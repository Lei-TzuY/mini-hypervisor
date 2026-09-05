use crate::config::VmConfig;
use crate::error::{Error, VmExitError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{
    LongModeBootLayout, LongModeConfigurationError, LONG_MODE_IDENTITY_MAP_SIZE,
    LONG_MODE_PAGE_SIZE, LONG_MODE_PML4_ADDR,
};
use crate::memory::{GuestMemory, GuestMemoryRegion, GuestPhysAddr};
use crate::portio::PortIoBus;
use crate::vcpu::{PortIoExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::fmt;

pub const LONG_MODE_INTERRUPT_GDT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x5000);
pub const LONG_MODE_INTERRUPT_IDT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x6000);
pub const LONG_MODE_INTERRUPT_TABLE_END: GuestPhysAddr = GuestPhysAddr::new(0x7000);
pub const LONG_MODE_INTERRUPT_VECTOR: u8 = 0x40;
pub const LONG_MODE_INTERRUPT_GUEST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
pub const LONG_MODE_INTERRUPT_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x1_1000);
pub const LONG_MODE_INTERRUPT_STACK_POINTER: u64 = 0x1f_f000;
pub const LONG_MODE_INTERRUPT_WINDOW_RIP: u64 = 0x1_0002;
pub const LONG_MODE_INTERRUPT_PROOF: &[u8; 2] = b"IM";
pub const LONG_MODE_INTERRUPT_TERMINAL_RIP: u64 = 0x1_0007;
pub const X86_RFLAGS_INTERRUPT_ENABLE: u64 = 1 << 9;

const X86_EXCEPTION_VECTOR_COUNT: u8 = 32;
const X86_INTERRUPT_GATE_SIZE: u64 = 16;
const X86_LONG_MODE_CODE_SELECTOR: u16 = 0x8;
const X86_INTERRUPT_GATE_PRESENT_RING0: u8 = 0x8e;
const X86_MAX_INTERRUPT_FRAME_BYTES: u64 = 5 * 8;
const LONG_MODE_INTERRUPT_EXIT_BUDGET: u32 = 3;
const GDT_LIMIT: u16 = 23;

const GDT_BYTES: [u8; 24] = [
    0, 0, 0, 0, 0, 0, 0, 0, // null descriptor
    0xff, 0xff, 0x00, 0x00, 0x00, 0x9b, 0xaf, 0x00, // ring-0 64-bit code descriptor
    0xff, 0xff, 0x00, 0x00, 0x00, 0x93, 0x8f, 0x00, // ring-0 data descriptor
];

const LONG_MODE_INTERRUPT_GUEST_BYTES: [u8; 7] = [
    0xfb, // sti -- enable maskable interrupts for the requested KVM interrupt window
    0x90, // nop -- complete STI's one-instruction interrupt shadow
    0xb0, b'M', // mov $'M', %al
    0xe6, 0xe9, // out %al, $0xe9
    0xf4, // hlt
];

const LONG_MODE_INTERRUPT_HANDLER_BYTES: [u8; 6] = [
    0xb0, b'I', // mov $'I', %al
    0xe6, 0xe9, // out %al, $0xe9
    0x48, 0xcf, // iretq
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LongModeInterruptGate {
    vector: u8,
    handler: GuestPhysAddr,
}

impl LongModeInterruptGate {
    #[must_use]
    pub const fn new(vector: u8, handler: GuestPhysAddr) -> Self {
        Self { vector, handler }
    }

    #[must_use]
    pub const fn vector(self) -> u8 {
        self.vector
    }

    #[must_use]
    pub const fn handler(self) -> GuestPhysAddr {
        self.handler
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LongModeInterruptConfigurationError {
    Boot(LongModeConfigurationError),
    NoInterruptGates,
    VectorReservedForExceptions {
        vector: u8,
    },
    DuplicateInterruptVector {
        vector: u8,
    },
    EntryOverlapsInterruptTables {
        entry: u64,
    },
    HandlerOutsideIdentityMap {
        handler: u64,
        mapped_size: u64,
    },
    HandlerOverlapsReservedTables {
        handler: u64,
    },
    InterruptStackFrameOutsideIdentityMap {
        stack_pointer: u64,
        frame_bytes: u64,
    },
    InterruptStackFrameOverlapsReservedTables {
        stack_pointer: u64,
        frame_start: u64,
    },
}

impl fmt::Display for LongModeInterruptConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boot(error) => error.fmt(f),
            Self::NoInterruptGates => write!(f, "long-mode interrupt layout requires at least one IDT gate"),
            Self::VectorReservedForExceptions { vector } => write!(
                f,
                "interrupt vector {vector:#x} is reserved by the x86 exception-vector range"
            ),
            Self::DuplicateInterruptVector { vector } => write!(
                f,
                "interrupt vector {vector:#x} is installed more than once"
            ),
            Self::EntryOverlapsInterruptTables { entry } => write!(
                f,
                "long-mode interrupt entry {entry:#x} overlaps the reserved GDT/IDT pages"
            ),
            Self::HandlerOutsideIdentityMap {
                handler,
                mapped_size,
            } => write!(
                f,
                "long-mode interrupt handler {handler:#x} is outside the identity-mapped range 0..{mapped_size:#x}"
            ),
            Self::HandlerOverlapsReservedTables { handler } => write!(
                f,
                "long-mode interrupt handler {handler:#x} overlaps bootstrap/GDT/IDT tables"
            ),
            Self::InterruptStackFrameOutsideIdentityMap {
                stack_pointer,
                frame_bytes,
            } => write!(
                f,
                "long-mode interrupt stack pointer {stack_pointer:#x} cannot reserve the bounded {frame_bytes}-byte interrupt frame"
            ),
            Self::InterruptStackFrameOverlapsReservedTables {
                stack_pointer,
                frame_start,
            } => write!(
                f,
                "long-mode interrupt stack frame {frame_start:#x}..{stack_pointer:#x} overlaps bootstrap/GDT/IDT tables"
            ),
        }
    }
}

impl std::error::Error for LongModeInterruptConfigurationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Boot(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LongModeConfigurationError> for LongModeInterruptConfigurationError {
    fn from(error: LongModeConfigurationError) -> Self {
        Self::Boot(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongModeInterruptLayout {
    boot: LongModeBootLayout,
    gates: Vec<LongModeInterruptGate>,
    idt_limit: u16,
}

impl LongModeInterruptLayout {
    pub fn new(
        memory: GuestMemoryRegion,
        entry: GuestPhysAddr,
        stack_pointer: u64,
        vector: u8,
        handler: GuestPhysAddr,
    ) -> Result<Self, LongModeInterruptConfigurationError> {
        Self::with_gates(
            memory,
            entry,
            stack_pointer,
            vec![LongModeInterruptGate::new(vector, handler)],
        )
    }

    pub fn with_gates(
        memory: GuestMemoryRegion,
        entry: GuestPhysAddr,
        stack_pointer: u64,
        gates: Vec<LongModeInterruptGate>,
    ) -> Result<Self, LongModeInterruptConfigurationError> {
        let boot = LongModeBootLayout::new(memory, entry, stack_pointer)?;
        if gates.is_empty() {
            return Err(LongModeInterruptConfigurationError::NoInterruptGates);
        }
        if is_interrupt_table_address(entry.get()) {
            return Err(
                LongModeInterruptConfigurationError::EntryOverlapsInterruptTables {
                    entry: entry.get(),
                },
            );
        }

        let Some(frame_start) = stack_pointer.checked_sub(X86_MAX_INTERRUPT_FRAME_BYTES) else {
            return Err(
                LongModeInterruptConfigurationError::InterruptStackFrameOutsideIdentityMap {
                    stack_pointer,
                    frame_bytes: X86_MAX_INTERRUPT_FRAME_BYTES,
                },
            );
        };
        if ranges_overlap(
            frame_start,
            stack_pointer,
            LONG_MODE_PML4_ADDR.get(),
            LONG_MODE_INTERRUPT_TABLE_END.get(),
        ) {
            return Err(
                LongModeInterruptConfigurationError::InterruptStackFrameOverlapsReservedTables {
                    stack_pointer,
                    frame_start,
                },
            );
        }

        let mut max_vector = 0_u8;
        for (index, gate) in gates.iter().copied().enumerate() {
            let vector = gate.vector();
            let handler = gate.handler();
            if vector < X86_EXCEPTION_VECTOR_COUNT {
                return Err(
                    LongModeInterruptConfigurationError::VectorReservedForExceptions { vector },
                );
            }
            if gates[..index]
                .iter()
                .any(|existing| existing.vector() == vector)
            {
                return Err(LongModeInterruptConfigurationError::DuplicateInterruptVector {
                    vector,
                });
            }
            if handler.get() >= LONG_MODE_IDENTITY_MAP_SIZE {
                return Err(
                    LongModeInterruptConfigurationError::HandlerOutsideIdentityMap {
                        handler: handler.get(),
                        mapped_size: LONG_MODE_IDENTITY_MAP_SIZE,
                    },
                );
            }
            if is_reserved_table_address(handler.get()) {
                return Err(
                    LongModeInterruptConfigurationError::HandlerOverlapsReservedTables {
                        handler: handler.get(),
                    },
                );
            }
            max_vector = max_vector.max(vector);
        }

        let idt_limit = u16::from(max_vector)
            .checked_add(1)
            .and_then(|entries| entries.checked_mul(X86_INTERRUPT_GATE_SIZE as u16))
            .and_then(|bytes| bytes.checked_sub(1))
            .expect("an 8-bit vector always fits in the 4 KiB x86 IDT");

        Ok(Self {
            boot,
            gates,
            idt_limit,
        })
    }

    #[must_use]
    pub const fn boot_layout(&self) -> &LongModeBootLayout {
        &self.boot
    }

    #[must_use]
    pub fn vector(&self) -> u8 {
        self.gates[0].vector()
    }

    #[must_use]
    pub fn handler(&self) -> GuestPhysAddr {
        self.gates[0].handler()
    }

    #[must_use]
    pub fn gates(&self) -> &[LongModeInterruptGate] {
        &self.gates
    }

    #[must_use]
    pub const fn gdt_base(&self) -> GuestPhysAddr {
        LONG_MODE_INTERRUPT_GDT_ADDR
    }

    #[must_use]
    pub const fn gdt_limit(&self) -> u16 {
        GDT_LIMIT
    }

    #[must_use]
    pub const fn idt_base(&self) -> GuestPhysAddr {
        LONG_MODE_INTERRUPT_IDT_ADDR
    }

    #[must_use]
    pub const fn idt_limit(&self) -> u16 {
        self.idt_limit
    }

    pub(crate) fn install_tables(&self, memory: &mut GuestMemory) -> Result<(), Error> {
        debug_assert_eq!(memory.region(), self.boot.memory());
        self.boot.install_page_tables(memory)?;

        let zero_page = [0_u8; LONG_MODE_PAGE_SIZE as usize];
        memory.write(LONG_MODE_INTERRUPT_GDT_ADDR, &zero_page)?;
        memory.write(LONG_MODE_INTERRUPT_IDT_ADDR, &zero_page)?;
        memory.write(LONG_MODE_INTERRUPT_GDT_ADDR, &GDT_BYTES)?;

        for gate in &self.gates {
            let gate_address = GuestPhysAddr::new(
                LONG_MODE_INTERRUPT_IDT_ADDR.get()
                    + u64::from(gate.vector()) * X86_INTERRUPT_GATE_SIZE,
            );
            memory.write(gate_address, &encode_interrupt_gate(gate.handler().get()))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongModeInterruptGuestResult {
    vector: u8,
    interrupt_window_rip: u64,
    interrupt_window_rflags: u64,
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
}

impl LongModeInterruptGuestResult {
    #[must_use]
    pub const fn vector(&self) -> u8 {
        self.vector
    }

    #[must_use]
    pub const fn interrupt_window_rip(&self) -> u64 {
        self.interrupt_window_rip
    }

    #[must_use]
    pub const fn interrupt_window_rflags(&self) -> u64 {
        self.interrupt_window_rflags
    }

    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    #[must_use]
    pub const fn report(&self) -> VmExitReport {
        self.report
    }
}

pub fn run_long_mode_interrupt_guest(
    config: VmConfig,
) -> Result<LongModeInterruptGuestResult, Error> {
    let guest = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_GUEST_ENTRY,
        LONG_MODE_INTERRUPT_GUEST_ENTRY,
        &LONG_MODE_INTERRUPT_GUEST_BYTES,
    )?;
    let handler = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_HANDLER,
        LONG_MODE_INTERRUPT_HANDLER,
        &LONG_MODE_INTERRUPT_HANDLER_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModeInterruptLayout::new(
        memory.region(),
        guest.entry(),
        LONG_MODE_INTERRUPT_STACK_POINTER,
        LONG_MODE_INTERRUPT_VECTOR,
        handler.entry(),
    )
    .expect("fixed deterministic long-mode interrupt fixture layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(&layout)?;
    let (interrupt_window_rip, interrupt_window_rflags) = vcpu.wait_for_interrupt_window()?;
    vcpu.inject_interrupt(layout.vector())?;

    let mut port_io = PortIoBus::with_debug_port();
    let execution =
        run_vcpu_until_stopped(&mut vcpu, &mut port_io, LONG_MODE_INTERRUPT_EXIT_BUDGET)?;
    if execution.io_exits().len() != LONG_MODE_INTERRUPT_PROOF.len() {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "long-mode direct interrupt proof output count",
            expected_reason: crate::vcpu::VcpuExit::Io.reason(),
            actual_reason: execution.report().exit().reason(),
        }));
    }

    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    Ok(LongModeInterruptGuestResult {
        vector: layout.vector(),
        interrupt_window_rip,
        interrupt_window_rflags,
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
    })
}

fn encode_interrupt_gate(handler: u64) -> [u8; 16] {
    let mut gate = [0_u8; 16];
    gate[0..2].copy_from_slice(&(handler as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&X86_LONG_MODE_CODE_SELECTOR.to_le_bytes());
    gate[4] = 0;
    gate[5] = X86_INTERRUPT_GATE_PRESENT_RING0;
    gate[6..8].copy_from_slice(&((handler >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((handler >> 32) as u32).to_le_bytes());
    gate
}

const fn is_interrupt_table_address(address: u64) -> bool {
    address >= LONG_MODE_INTERRUPT_GDT_ADDR.get() && address < LONG_MODE_INTERRUPT_TABLE_END.get()
}

const fn is_reserved_table_address(address: u64) -> bool {
    address >= LONG_MODE_PML4_ADDR.get() && address < LONG_MODE_INTERRUPT_TABLE_END.get()
}

const fn ranges_overlap(start: u64, end: u64, other_start: u64, other_end: u64) -> bool {
    start < other_end && other_start < end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_region() -> GuestMemoryRegion {
        GuestMemoryRegion::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap()
    }

    fn layout() -> LongModeInterruptLayout {
        LongModeInterruptLayout::new(
            memory_region(),
            LONG_MODE_INTERRUPT_GUEST_ENTRY,
            LONG_MODE_INTERRUPT_STACK_POINTER,
            LONG_MODE_INTERRUPT_VECTOR,
            LONG_MODE_INTERRUPT_HANDLER,
        )
        .unwrap()
    }

    #[test]
    fn installs_exact_gdt_and_idt_interrupt_gate() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let layout = layout();
        layout.install_tables(&mut memory).unwrap();

        let mut gdt = [0_u8; GDT_BYTES.len()];
        memory.read(LONG_MODE_INTERRUPT_GDT_ADDR, &mut gdt).unwrap();
        assert_eq!(gdt, GDT_BYTES);
        assert_eq!(layout.gdt_limit(), 23);

        let gate_address = GuestPhysAddr::new(
            LONG_MODE_INTERRUPT_IDT_ADDR.get()
                + u64::from(LONG_MODE_INTERRUPT_VECTOR) * X86_INTERRUPT_GATE_SIZE,
        );
        let mut gate = [0_u8; 16];
        memory.read(gate_address, &mut gate).unwrap();
        assert_eq!(
            gate,
            encode_interrupt_gate(LONG_MODE_INTERRUPT_HANDLER.get())
        );
        assert_eq!(
            gate,
            [
                0x00, 0x10, 0x08, 0x00, 0x00, 0x8e, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00,
            ]
        );
        assert_eq!(layout.idt_limit(), 0x40f);
        assert_eq!(
            layout.gates(),
            &[LongModeInterruptGate::new(
                LONG_MODE_INTERRUPT_VECTOR,
                LONG_MODE_INTERRUPT_HANDLER
            )]
        );
    }

    #[test]
    fn installs_multiple_interrupt_gates_and_expands_idt_limit() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let second_handler = GuestPhysAddr::new(0x1_2000);
        let gates = vec![
            LongModeInterruptGate::new(0x40, LONG_MODE_INTERRUPT_HANDLER),
            LongModeInterruptGate::new(0x41, second_handler),
        ];
        let layout = LongModeInterruptLayout::with_gates(
            memory.region(),
            LONG_MODE_INTERRUPT_GUEST_ENTRY,
            LONG_MODE_INTERRUPT_STACK_POINTER,
            gates.clone(),
        )
        .unwrap();
        layout.install_tables(&mut memory).unwrap();

        for gate in &gates {
            let address = GuestPhysAddr::new(
                LONG_MODE_INTERRUPT_IDT_ADDR.get()
                    + u64::from(gate.vector()) * X86_INTERRUPT_GATE_SIZE,
            );
            let mut encoded = [0_u8; 16];
            memory.read(address, &mut encoded).unwrap();
            assert_eq!(encoded, encode_interrupt_gate(gate.handler().get()));
        }
        assert_eq!(layout.gates(), gates.as_slice());
        assert_eq!(layout.idt_limit(), 0x41f);
    }

    #[test]
    fn rejects_empty_and_duplicate_interrupt_gate_sets() {
        assert!(matches!(
            LongModeInterruptLayout::with_gates(
                memory_region(),
                LONG_MODE_INTERRUPT_GUEST_ENTRY,
                LONG_MODE_INTERRUPT_STACK_POINTER,
                vec![]
            ),
            Err(LongModeInterruptConfigurationError::NoInterruptGates)
        ));
        assert!(matches!(
            LongModeInterruptLayout::with_gates(
                memory_region(),
                LONG_MODE_INTERRUPT_GUEST_ENTRY,
                LONG_MODE_INTERRUPT_STACK_POINTER,
                vec![
                    LongModeInterruptGate::new(0x40, LONG_MODE_INTERRUPT_HANDLER),
                    LongModeInterruptGate::new(0x40, GuestPhysAddr::new(0x1_2000)),
                ]
            ),
            Err(LongModeInterruptConfigurationError::DuplicateInterruptVector { vector: 0x40 })
        ));
    }

    #[test]
    fn rejects_exception_vectors_and_reserved_table_collisions() {
        assert!(matches!(
            LongModeInterruptLayout::new(
                memory_region(),
                LONG_MODE_INTERRUPT_GUEST_ENTRY,
                LONG_MODE_INTERRUPT_STACK_POINTER,
                0x1f,
                LONG_MODE_INTERRUPT_HANDLER,
            ),
            Err(LongModeInterruptConfigurationError::VectorReservedForExceptions { vector: 0x1f })
        ));
        assert!(matches!(
            LongModeInterruptLayout::new(
                memory_region(),
                LONG_MODE_INTERRUPT_GDT_ADDR,
                LONG_MODE_INTERRUPT_STACK_POINTER,
                LONG_MODE_INTERRUPT_VECTOR,
                LONG_MODE_INTERRUPT_HANDLER,
            ),
            Err(LongModeInterruptConfigurationError::EntryOverlapsInterruptTables { .. })
        ));
        assert!(matches!(
            LongModeInterruptLayout::new(
                memory_region(),
                LONG_MODE_INTERRUPT_GUEST_ENTRY,
                LONG_MODE_INTERRUPT_STACK_POINTER,
                LONG_MODE_INTERRUPT_VECTOR,
                LONG_MODE_INTERRUPT_IDT_ADDR,
            ),
            Err(LongModeInterruptConfigurationError::HandlerOverlapsReservedTables { .. })
        ));
    }

    #[test]
    fn rejects_handler_outside_identity_map_and_stack_frame_overlaps() {
        assert!(matches!(
            LongModeInterruptLayout::new(
                memory_region(),
                LONG_MODE_INTERRUPT_GUEST_ENTRY,
                LONG_MODE_INTERRUPT_STACK_POINTER,
                LONG_MODE_INTERRUPT_VECTOR,
                GuestPhysAddr::new(LONG_MODE_IDENTITY_MAP_SIZE),
            ),
            Err(LongModeInterruptConfigurationError::HandlerOutsideIdentityMap { .. })
        ));
        assert!(matches!(
            LongModeInterruptLayout::new(
                memory_region(),
                LONG_MODE_INTERRUPT_GUEST_ENTRY,
                LONG_MODE_INTERRUPT_TABLE_END.get() + 8,
                LONG_MODE_INTERRUPT_VECTOR,
                LONG_MODE_INTERRUPT_HANDLER,
            ),
            Err(
                LongModeInterruptConfigurationError::InterruptStackFrameOverlapsReservedTables { .. }
            )
        ));
    }

    #[test]
    fn deterministic_guest_and_handler_machine_code_are_stable() {
        assert_eq!(
            LONG_MODE_INTERRUPT_GUEST_BYTES,
            [0xfb, 0x90, 0xb0, b'M', 0xe6, 0xe9, 0xf4]
        );
        assert_eq!(
            LONG_MODE_INTERRUPT_HANDLER_BYTES,
            [0xb0, b'I', 0xe6, 0xe9, 0x48, 0xcf]
        );
        assert_eq!(LONG_MODE_INTERRUPT_PROOF, b"IM");
        assert_eq!(
            LONG_MODE_INTERRUPT_GUEST_ENTRY.get() + 2,
            LONG_MODE_INTERRUPT_WINDOW_RIP
        );
        assert_eq!(
            LONG_MODE_INTERRUPT_GUEST_ENTRY.get() + LONG_MODE_INTERRUPT_GUEST_BYTES.len() as u64,
            LONG_MODE_INTERRUPT_TERMINAL_RIP
        );
    }
}
