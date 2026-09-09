use crate::address_space::{
    AddressSpaceSwitchLayout, ADDRESS_SPACE_A_CR3, ADDRESS_SPACE_B_PML4_ADDR,
    ADDRESS_SPACE_B_PT_ADDR, ADDRESS_SPACE_B_USER_CODE_BACKING,
};
use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::{PortIoBus, DEBUG_PORT};
use crate::privilege::{
    PRIVILEGE_KERNEL_ENTRY, PRIVILEGE_PT_ADDR, PRIVILEGE_RETURN_HANDLER,
    PRIVILEGE_TERMINAL_HANDLER, PRIVILEGE_USER_ENTRY,
};
use crate::vcpu::{PortIoDirection, PortIoExit, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;

pub const TASK_CONTEXT_PAGE_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x30000);
pub const TASK_A_CONTEXT_ADDR: GuestPhysAddr = TASK_CONTEXT_PAGE_ADDR;
pub const TASK_B_CONTEXT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x30040);
pub const TASK_TERMINAL_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x30080);
pub const TASK_A_INITIAL_RSP: u64 = 0x1fcff0;
pub const TASK_B_INITIAL_RSP: u64 = 0x1fcfd0;
pub const TASK_A_STACK_MARKER_PHYS: GuestPhysAddr = GuestPhysAddr::new(0x1fcfe8);
pub const TASK_B_STACK_MARKER_PHYS: GuestPhysAddr = GuestPhysAddr::new(0x22fc8);
pub const TASK_A_R12: u64 = 0x1111;
pub const TASK_B_INITIAL_R12: u64 = 0x2222;
pub const TASK_B_SAVED_R12: u64 = 0x2223;
pub const TASK_A_SAVED_RIP: u64 = 0x11011;
pub const TASK_B_SAVED_RIP: u64 = 0x11013;
pub const TASK_TERMINAL_USER_RIP: u64 = 0x11013;
pub const TASK_TERMINAL_RIP: u64 = 0x13059;
pub const TASK_CONTEXT_PROOF: &[u8; 4] = b"ABRD";

const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITABLE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_PAGE_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const TASK_EXIT_BUDGET: u32 = 5;
const TASK_CONTEXT_BYTES: usize = 48;
const TASK_TERMINAL_OBSERVATION_BYTES: usize = 32;
const TASK_USER_RFLAGS: u64 = 0x202;

const KERNEL_BOOT_BYTES: [u8; 41] = [
    0xfa, 0x66, 0xb8, 0x28, 0x00, 0x0f, 0x00, 0xd8, 0x6a, 0x1b, 0x48, 0xb8, 0xf0, 0xcf, 0x1f, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x50, 0x68, 0x02, 0x02, 0x00, 0x00, 0x6a, 0x23, 0x48, 0xb8, 0x00, 0x10,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x50, 0x48, 0xcf,
];

const TASK_A_BYTES: [u8; 19] = [
    0x49, 0xbc, 0x11, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc6, 0x44, 0x24, 0xf8, 0x61, 0xcd,
    0x80, 0xcd, 0x81,
];

const TASK_B_BYTES: [u8; 21] = [
    0x49, 0x81, 0xfc, 0x22, 0x22, 0x00, 0x00, 0x75, 0x0a, 0xc6, 0x44, 0x24, 0xf8, 0x62, 0x49, 0xff,
    0xc4, 0xcd, 0x80, 0xcd, 0x81,
];

const SCHEDULER_HANDLER_BYTES: [u8; 196] = [
    0x0f, 0x20, 0xdb, 0x48, 0x81, 0xfb, 0x00, 0x10, 0x00, 0x00, 0x74, 0x0e, 0x48, 0x81, 0xfb, 0x00,
    0xb0, 0x00, 0x00, 0x74, 0x2c, 0xe9, 0xa5, 0x00, 0x00, 0x00, 0x48, 0xbf, 0x00, 0x00, 0x03, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x48, 0xbe, 0x40, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x49, 0x81,
    0xfc, 0x11, 0x11, 0x00, 0x00, 0x0f, 0x85, 0x84, 0x00, 0x00, 0x00, 0xb0, 0x41, 0xe6, 0xe9, 0xeb,
    0x21, 0x48, 0xbf, 0x40, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0xbe, 0x00, 0x00, 0x03,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x49, 0x81, 0xfc, 0x23, 0x22, 0x00, 0x00, 0x75, 0x61, 0xb0, 0x42,
    0xe6, 0xe9, 0x48, 0x89, 0x1f, 0x48, 0x8b, 0x04, 0x24, 0x48, 0x89, 0x47, 0x08, 0x48, 0x8b, 0x44,
    0x24, 0x18, 0x48, 0x89, 0x47, 0x10, 0x48, 0x8b, 0x44, 0x24, 0x10, 0x48, 0x89, 0x47, 0x18, 0x4c,
    0x89, 0x67, 0x20, 0x48, 0xff, 0x47, 0x28, 0x48, 0x8b, 0x46, 0x08, 0x48, 0x89, 0x04, 0x24, 0x48,
    0xc7, 0x44, 0x24, 0x08, 0x23, 0x00, 0x00, 0x00, 0x48, 0x8b, 0x46, 0x18, 0x48, 0x89, 0x44, 0x24,
    0x10, 0x48, 0x8b, 0x46, 0x10, 0x48, 0x89, 0x44, 0x24, 0x18, 0x48, 0xc7, 0x44, 0x24, 0x20, 0x1b,
    0x00, 0x00, 0x00, 0x4c, 0x8b, 0x66, 0x20, 0x48, 0x8b, 0x06, 0x0f, 0x22, 0xd8, 0x48, 0xcf, 0xb0,
    0x46, 0xe6, 0xe9, 0xf4,
];

const TERMINAL_HANDLER_BYTES: [u8; 94] = [
    0x0f, 0x20, 0xdb, 0x48, 0x81, 0xfb, 0x00, 0x10, 0x00, 0x00, 0x75, 0x4d, 0x49, 0x81, 0xfc, 0x11,
    0x11, 0x00, 0x00, 0x75, 0x44, 0x48, 0x8b, 0x04, 0x24, 0x48, 0x3d, 0x13, 0x10, 0x01, 0x00, 0x75,
    0x38, 0x48, 0x8b, 0x44, 0x24, 0x18, 0x48, 0x3d, 0xf0, 0xcf, 0x1f, 0x00, 0x75, 0x2b, 0x48, 0xbf,
    0x80, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x89, 0x1f, 0x48, 0x8b, 0x04, 0x24, 0x48,
    0x89, 0x47, 0x08, 0x48, 0x8b, 0x44, 0x24, 0x18, 0x48, 0x89, 0x47, 0x10, 0x4c, 0x89, 0x67, 0x18,
    0xb0, 0x52, 0xe6, 0xe9, 0xb0, 0x44, 0xe6, 0xe9, 0xf4, 0xb0, 0x46, 0xe6, 0xe9, 0xf4,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskContextSnapshot {
    cr3: u64,
    rip: u64,
    rsp: u64,
    rflags: u64,
    r12: u64,
    save_count: u64,
}

impl TaskContextSnapshot {
    #[must_use]
    pub const fn cr3(self) -> u64 {
        self.cr3
    }
    #[must_use]
    pub const fn rip(self) -> u64 {
        self.rip
    }
    #[must_use]
    pub const fn rsp(self) -> u64 {
        self.rsp
    }
    #[must_use]
    pub const fn rflags(self) -> u64 {
        self.rflags
    }
    #[must_use]
    pub const fn r12(self) -> u64 {
        self.r12
    }
    #[must_use]
    pub const fn save_count(self) -> u64 {
        self.save_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskTerminalObservation {
    cr3: u64,
    rip: u64,
    rsp: u64,
    r12: u64,
}

impl TaskTerminalObservation {
    #[must_use]
    pub const fn cr3(self) -> u64 {
        self.cr3
    }
    #[must_use]
    pub const fn rip(self) -> u64 {
        self.rip
    }
    #[must_use]
    pub const fn rsp(self) -> u64 {
        self.rsp
    }
    #[must_use]
    pub const fn r12(self) -> u64 {
        self.r12
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskContextSwitchGuestResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
    task_a: TaskContextSnapshot,
    task_b: TaskContextSnapshot,
    terminal: TaskTerminalObservation,
    final_cr3: u64,
    final_r12: u64,
    task_a_stack_marker: u8,
    task_b_stack_marker: u8,
    first_context_pte: u64,
    second_context_pte: u64,
}

impl TaskContextSwitchGuestResult {
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
    #[must_use]
    pub const fn task_a(&self) -> TaskContextSnapshot {
        self.task_a
    }
    #[must_use]
    pub const fn task_b(&self) -> TaskContextSnapshot {
        self.task_b
    }
    #[must_use]
    pub const fn terminal(&self) -> TaskTerminalObservation {
        self.terminal
    }
    #[must_use]
    pub const fn final_cr3(&self) -> u64 {
        self.final_cr3
    }
    #[must_use]
    pub const fn final_r12(&self) -> u64 {
        self.final_r12
    }
    #[must_use]
    pub const fn task_a_stack_marker(&self) -> u8 {
        self.task_a_stack_marker
    }
    #[must_use]
    pub const fn task_b_stack_marker(&self) -> u8 {
        self.task_b_stack_marker
    }
    #[must_use]
    pub const fn first_context_pte(&self) -> u64 {
        self.first_context_pte
    }
    #[must_use]
    pub const fn second_context_pte(&self) -> u64 {
        self.second_context_pte
    }
}

pub fn run_task_context_switch_guest(
    config: VmConfig,
) -> Result<TaskContextSwitchGuestResult, Error> {
    let kernel = FlatGuestImage::new(
        PRIVILEGE_KERNEL_ENTRY,
        PRIVILEGE_KERNEL_ENTRY,
        &KERNEL_BOOT_BYTES,
    )?;
    let task_a = FlatGuestImage::new(PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_ENTRY, &TASK_A_BYTES)?;
    let task_b = FlatGuestImage::new(
        ADDRESS_SPACE_B_USER_CODE_BACKING,
        ADDRESS_SPACE_B_USER_CODE_BACKING,
        &TASK_B_BYTES,
    )?;
    let scheduler = FlatGuestImage::new(
        PRIVILEGE_RETURN_HANDLER,
        PRIVILEGE_RETURN_HANDLER,
        &SCHEDULER_HANDLER_BYTES,
    )?;
    let terminal = FlatGuestImage::new(
        PRIVILEGE_TERMINAL_HANDLER,
        PRIVILEGE_TERMINAL_HANDLER,
        &TERMINAL_HANDLER_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = AddressSpaceSwitchLayout::new(memory.region())
        .expect("fixed bounded task-context layout remains valid");
    layout.install_tables(&mut memory)?;
    kernel.load(&mut memory)?;
    task_a.load(&mut memory)?;
    task_b.load(&mut memory)?;
    scheduler.load(&mut memory)?;
    terminal.load(&mut memory)?;
    memory.write(TASK_CONTEXT_PAGE_ADDR, &[0; LONG_MODE_PAGE_SIZE as usize])?;
    write_context(
        &mut memory,
        TASK_B_CONTEXT_ADDR,
        TaskContextSnapshot {
            cr3: ADDRESS_SPACE_B_PML4_ADDR.get(),
            rip: PRIVILEGE_USER_ENTRY.get(),
            rsp: TASK_B_INITIAL_RSP,
            rflags: TASK_USER_RFLAGS,
            r12: TASK_B_INITIAL_R12,
            save_count: 0,
        },
    )?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(layout.privilege_layout())?;
    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, TASK_EXIT_BUDGET)?;

    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != TASK_CONTEXT_PROOF
        || execution.io_exits().len() != TASK_CONTEXT_PROOF.len()
    {
        return Err(verification_error(
            "task context proof",
            format!(
                "expected proof {:?} across {} exits, got {:?} across {} exits",
                TASK_CONTEXT_PROOF,
                TASK_CONTEXT_PROOF.len(),
                proof,
                execution.io_exits().len()
            ),
        ));
    }
    for (io, expected) in execution
        .io_exits()
        .iter()
        .zip(TASK_CONTEXT_PROOF.iter().copied())
    {
        if io.direction() != PortIoDirection::Out
            || io.port() != DEBUG_PORT
            || io.size() != 1
            || io.count() != 1
            || io.output_data() != [expected]
        {
            return Err(verification_error(
                "task context debug-port exit",
                format!("unexpected I/O exit {io:?}, expected byte {expected:#x}"),
            ));
        }
    }

    let final_regs = vcpu.capture_register_snapshot()?;
    let final_special = vcpu.capture_special_register_snapshot()?;
    let guest_memory = vm
        .guest_memory()
        .expect("registered task memory remains VM-owned");
    let task_a_context = read_context(guest_memory, TASK_A_CONTEXT_ADDR)?;
    let task_b_context = read_context(guest_memory, TASK_B_CONTEXT_ADDR)?;
    let terminal_observation = read_terminal_observation(guest_memory)?;
    let task_a_stack_marker = read_byte(guest_memory, TASK_A_STACK_MARKER_PHYS)?;
    let task_b_stack_marker = read_byte(guest_memory, TASK_B_STACK_MARKER_PHYS)?;
    let first_context_pte = read_pte(
        guest_memory,
        PRIVILEGE_PT_ADDR,
        TASK_CONTEXT_PAGE_ADDR.get(),
    )?;
    let second_context_pte = read_pte(
        guest_memory,
        ADDRESS_SPACE_B_PT_ADDR,
        TASK_CONTEXT_PAGE_ADDR.get(),
    )?;

    validate_result(
        execution.report(),
        task_a_context,
        task_b_context,
        terminal_observation,
        final_special.cr3(),
        final_regs.r12(),
        task_a_stack_marker,
        task_b_stack_marker,
        first_context_pte,
        second_context_pte,
    )?;

    Ok(TaskContextSwitchGuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
        task_a: task_a_context,
        task_b: task_b_context,
        terminal: terminal_observation,
        final_cr3: final_special.cr3(),
        final_r12: final_regs.r12(),
        task_a_stack_marker,
        task_b_stack_marker,
        first_context_pte,
        second_context_pte,
    })
}

fn write_context(
    memory: &mut GuestMemory,
    address: GuestPhysAddr,
    context: TaskContextSnapshot,
) -> Result<(), Error> {
    let values = [
        context.cr3,
        context.rip,
        context.rsp,
        context.rflags,
        context.r12,
        context.save_count,
    ];
    for (index, value) in values.into_iter().enumerate() {
        memory.write(
            GuestPhysAddr::new(address.get() + index as u64 * 8),
            &value.to_le_bytes(),
        )?;
    }
    Ok(())
}

fn read_context(
    memory: &GuestMemory,
    address: GuestPhysAddr,
) -> Result<TaskContextSnapshot, Error> {
    let mut bytes = [0_u8; TASK_CONTEXT_BYTES];
    memory.read(address, &mut bytes)?;
    Ok(TaskContextSnapshot {
        cr3: field(&bytes, 0),
        rip: field(&bytes, 8),
        rsp: field(&bytes, 16),
        rflags: field(&bytes, 24),
        r12: field(&bytes, 32),
        save_count: field(&bytes, 40),
    })
}

fn read_terminal_observation(memory: &GuestMemory) -> Result<TaskTerminalObservation, Error> {
    let mut bytes = [0_u8; TASK_TERMINAL_OBSERVATION_BYTES];
    memory.read(TASK_TERMINAL_OBSERVATION_ADDR, &mut bytes)?;
    Ok(TaskTerminalObservation {
        cr3: field(&bytes, 0),
        rip: field(&bytes, 8),
        rsp: field(&bytes, 16),
        r12: field(&bytes, 24),
    })
}

fn field(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("fixed task field"),
    )
}

fn read_byte(memory: &GuestMemory, address: GuestPhysAddr) -> Result<u8, Error> {
    let mut byte = [0_u8; 1];
    memory.read(address, &mut byte)?;
    Ok(byte[0])
}

fn read_pte(
    memory: &GuestMemory,
    table: GuestPhysAddr,
    virtual_address: u64,
) -> Result<u64, Error> {
    let index = (virtual_address & !(LONG_MODE_PAGE_SIZE - 1)) / LONG_MODE_PAGE_SIZE;
    let mut bytes = [0_u8; 8];
    memory.read(GuestPhysAddr::new(table.get() + index * 8), &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

#[allow(clippy::too_many_arguments)]
fn validate_result(
    report: VmExitReport,
    task_a: TaskContextSnapshot,
    task_b: TaskContextSnapshot,
    terminal: TaskTerminalObservation,
    final_cr3: u64,
    final_r12: u64,
    task_a_stack_marker: u8,
    task_b_stack_marker: u8,
    first_context_pte: u64,
    second_context_pte: u64,
) -> Result<(), Error> {
    if report.exit() != VcpuExit::Hlt
        || report.rip() != TASK_TERMINAL_RIP
        || report.rflags() & 0x2 != 0x2
    {
        return Err(verification_error(
            "task context terminal exit",
            format!("expected HLT at {TASK_TERMINAL_RIP:#x} with RFLAGS bit1, got {report}"),
        ));
    }
    let expected_a = TaskContextSnapshot {
        cr3: ADDRESS_SPACE_A_CR3.get(),
        rip: TASK_A_SAVED_RIP,
        rsp: TASK_A_INITIAL_RSP,
        rflags: TASK_USER_RFLAGS,
        r12: TASK_A_R12,
        save_count: 1,
    };
    let expected_b = TaskContextSnapshot {
        cr3: ADDRESS_SPACE_B_PML4_ADDR.get(),
        rip: TASK_B_SAVED_RIP,
        rsp: TASK_B_INITIAL_RSP,
        rflags: TASK_USER_RFLAGS,
        r12: TASK_B_SAVED_R12,
        save_count: 1,
    };
    let expected_terminal = TaskTerminalObservation {
        cr3: ADDRESS_SPACE_A_CR3.get(),
        rip: TASK_TERMINAL_USER_RIP,
        rsp: TASK_A_INITIAL_RSP,
        r12: TASK_A_R12,
    };
    if task_a != expected_a || task_b != expected_b || terminal != expected_terminal {
        return Err(verification_error(
            "task context ownership",
            format!("expected A {expected_a:?}, B {expected_b:?}, terminal {expected_terminal:?}; got A {task_a:?}, B {task_b:?}, terminal {terminal:?}"),
        ));
    }
    if final_cr3 != ADDRESS_SPACE_A_CR3.get() || final_r12 != TASK_A_R12 {
        return Err(verification_error(
            "task final register ownership",
            format!(
                "expected CR3={:#x}, R12={:#x}; got CR3={final_cr3:#x}, R12={final_r12:#x}",
                ADDRESS_SPACE_A_CR3.get(),
                TASK_A_R12
            ),
        ));
    }
    if task_a_stack_marker != b'a' || task_b_stack_marker != b'b' {
        return Err(verification_error(
            "task stack ownership",
            format!("expected physical stack markers a/b, got {task_a_stack_marker:#x}/{task_b_stack_marker:#x}"),
        ));
    }
    for (role, pte) in [
        ("A task-context page", first_context_pte),
        ("B task-context page", second_context_pte),
    ] {
        if pte & X86_PAGE_ADDRESS_MASK != TASK_CONTEXT_PAGE_ADDR.get()
            || pte & (X86_PAGE_PRESENT | X86_PAGE_WRITABLE)
                != (X86_PAGE_PRESENT | X86_PAGE_WRITABLE)
            || pte & X86_PAGE_USER != 0
        {
            return Err(verification_error(
                "task context PTE ownership",
                format!(
                    "{role}: expected supervisor P/W mapping to {:#x}, got {pte:#x}",
                    TASK_CONTEXT_PAGE_ADDR.get()
                ),
            ));
        }
    }
    Ok(())
}

fn verification_error(stage: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation: stage,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

const _: () = {
    assert!(TASK_CONTEXT_PAGE_ADDR.get() % LONG_MODE_PAGE_SIZE == 0);
    assert!(
        TASK_TERMINAL_OBSERVATION_ADDR.get() + (TASK_TERMINAL_OBSERVATION_BYTES as u64)
            < TASK_CONTEXT_PAGE_ADDR.get() + LONG_MODE_PAGE_SIZE
    );
    assert!(TASK_B_STACK_MARKER_PHYS.get() < LONG_MODE_IDENTITY_MAP_SIZE);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_task_context_page_is_supervisor_owned_in_both_roots() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let layout = AddressSpaceSwitchLayout::new(memory.region()).unwrap();
        layout.install_tables(&mut memory).unwrap();
        let first = read_pte(&memory, PRIVILEGE_PT_ADDR, TASK_CONTEXT_PAGE_ADDR.get()).unwrap();
        let second = read_pte(
            &memory,
            ADDRESS_SPACE_B_PT_ADDR,
            TASK_CONTEXT_PAGE_ADDR.get(),
        )
        .unwrap();
        for pte in [first, second] {
            assert_eq!(pte & X86_PAGE_ADDRESS_MASK, TASK_CONTEXT_PAGE_ADDR.get());
            assert_eq!(pte & X86_PAGE_PRESENT, X86_PAGE_PRESENT);
            assert_eq!(pte & X86_PAGE_WRITABLE, X86_PAGE_WRITABLE);
            assert_eq!(pte & X86_PAGE_USER, 0);
        }
    }

    #[test]
    fn context_encoding_round_trips_exact_fields() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let expected = TaskContextSnapshot {
            cr3: 0xb000,
            rip: 0x11000,
            rsp: TASK_B_INITIAL_RSP,
            rflags: TASK_USER_RFLAGS,
            r12: TASK_B_INITIAL_R12,
            save_count: 7,
        };
        write_context(&mut memory, TASK_B_CONTEXT_ADDR, expected).unwrap();
        assert_eq!(
            read_context(&memory, TASK_B_CONTEXT_ADDR).unwrap(),
            expected
        );
    }

    #[test]
    fn machine_code_preserves_exact_task_continuation_contract() {
        assert_eq!(TASK_A_BYTES.len(), 19);
        assert_eq!(TASK_B_BYTES.len(), 21);
        assert_eq!(SCHEDULER_HANDLER_BYTES.len(), 196);
        assert_eq!(TERMINAL_HANDLER_BYTES.len(), 94);
        assert_eq!(&TASK_A_BYTES[15..19], &[0xcd, 0x80, 0xcd, 0x81]);
        assert_eq!(PRIVILEGE_USER_ENTRY.get() + 17, TASK_A_SAVED_RIP);
        assert_eq!(PRIVILEGE_USER_ENTRY.get() + 19, TASK_TERMINAL_USER_RIP);
        assert_eq!(PRIVILEGE_USER_ENTRY.get() + 19, TASK_B_SAVED_RIP);
        assert_eq!(TERMINAL_HANDLER_BYTES[88], 0xf4);
    }
}

mod preemption;
pub use preemption::*;
mod wakeup;
pub use wakeup::*;