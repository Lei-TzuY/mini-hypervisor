use super::MmioBus;
use crate::config::VmConfig;
use crate::error::{Error, VmExitError};
use crate::execution::run_vcpu_until_stopped_with_mmio;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{
    LongModeBootLayout, LongModeConfigurationError, LONG_MODE_ALIAS_PT_ADDR,
    LONG_MODE_ALIAS_VIRTUAL_BASE, LONG_MODE_ALIAS_VIRTUAL_END, LONG_MODE_IDENTITY_MAP_SIZE,
    LONG_MODE_PAGE_SIZE, LONG_MODE_PD_ADDR,
};
use crate::memory::{GuestMemory, GuestMemoryRegion, GuestPhysAddr};
use crate::portio::PortIoBus;
use crate::vcpu::{MmioExit, PortIoExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::fmt;

pub const LONG_MODE_MMIO_VIRTUAL_PAGE: u64 = 0x50_0000;
pub const LONG_MODE_MMIO_DEVICE_GPA: u64 = 0x1000_0000;
pub const LONG_MODE_MMIO_GUEST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
pub const LONG_MODE_MMIO_STACK_POINTER: u64 = 0x1f_f000;
pub const LONG_MODE_MMIO_READ_VALUE: u8 = b'R';
pub const LONG_MODE_MMIO_WRITE_VALUE: u8 = b'W';
pub const LONG_MODE_MMIO_PROOF: &[u8; 4] = b"R64M";
pub const LONG_MODE_MMIO_TERMINAL_RIP: u64 = 0x1_001e;

const PAGE_TABLE_ENTRY_FLAGS: u64 = 0x3;
const ALIAS_PD_INDEX: u64 = LONG_MODE_ALIAS_VIRTUAL_BASE / LONG_MODE_IDENTITY_MAP_SIZE;
const LONG_MODE_MMIO_EXIT_BUDGET: u32 = 7;

const LONG_MODE_MMIO_GUEST_BYTES: [u8; 30] = [
    0x48, 0xbb, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs $0x500000, %rbx
    0xc6, 0x03, b'W', // movb $'W', (%rbx) -- virtual MMIO write
    0x8a, 0x03, // mov (%rbx), %al -- virtual MMIO read
    0xe6, 0xe9, // out 0xe9, al -- 'R'
    0xb0, b'6', 0xe6, 0xe9, // output '6'
    0xb0, b'4', 0xe6, 0xe9, // output '4'
    0xb0, b'M', 0xe6, 0xe9, // output 'M'
    0xf4, // hlt
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LongModeMmioPageMapping {
    virtual_page: u64,
    device_gpa: u64,
}

impl LongModeMmioPageMapping {
    #[must_use]
    pub const fn new(virtual_page: u64, device_gpa: u64) -> Self {
        Self {
            virtual_page,
            device_gpa,
        }
    }

    #[must_use]
    pub const fn virtual_page(self) -> u64 {
        self.virtual_page
    }

    #[must_use]
    pub const fn device_gpa(self) -> u64 {
        self.device_gpa
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LongModeMmioConfigurationError {
    Boot(LongModeConfigurationError),
    NoDeviceMappings,
    VirtualPageMisaligned { virtual_page: u64 },
    VirtualPageOutsideAliasWindow { virtual_page: u64 },
    DevicePageMisaligned { device_page: u64 },
    DevicePageAddressOverflow { device_page: u64 },
    DevicePageBackedByRam { device_page: u64, ram_end: u64 },
    DuplicateVirtualPage { virtual_page: u64 },
}

impl fmt::Display for LongModeMmioConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boot(error) => error.fmt(f),
            Self::NoDeviceMappings => write!(f, "long-mode MMIO layout requires at least one device mapping"),
            Self::VirtualPageMisaligned { virtual_page } => write!(
                f,
                "long-mode MMIO virtual page {virtual_page:#x} is not 4 KiB aligned"
            ),
            Self::VirtualPageOutsideAliasWindow { virtual_page } => write!(
                f,
                "long-mode MMIO virtual page {virtual_page:#x} is outside {LONG_MODE_ALIAS_VIRTUAL_BASE:#x}..{LONG_MODE_ALIAS_VIRTUAL_END:#x}"
            ),
            Self::DevicePageMisaligned { device_page } => write!(
                f,
                "long-mode MMIO device page {device_page:#x} is not 4 KiB aligned"
            ),
            Self::DevicePageAddressOverflow { device_page } => write!(
                f,
                "long-mode MMIO device page {device_page:#x} overflows the guest physical address space"
            ),
            Self::DevicePageBackedByRam {
                device_page,
                ram_end,
            } => write!(
                f,
                "long-mode MMIO device page {device_page:#x} must remain outside registered RAM ending at {ram_end:#x}"
            ),
            Self::DuplicateVirtualPage { virtual_page } => write!(
                f,
                "long-mode MMIO virtual page {virtual_page:#x} is mapped more than once"
            ),
        }
    }
}

impl std::error::Error for LongModeMmioConfigurationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Boot(error) => Some(error),
            Self::NoDeviceMappings
            | Self::VirtualPageMisaligned { .. }
            | Self::VirtualPageOutsideAliasWindow { .. }
            | Self::DevicePageMisaligned { .. }
            | Self::DevicePageAddressOverflow { .. }
            | Self::DevicePageBackedByRam { .. }
            | Self::DuplicateVirtualPage { .. } => None,
        }
    }
}

impl From<LongModeConfigurationError> for LongModeMmioConfigurationError {
    fn from(error: LongModeConfigurationError) -> Self {
        Self::Boot(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongModeMmioBootLayout {
    boot: LongModeBootLayout,
    device_mappings: Vec<LongModeMmioPageMapping>,
}

impl LongModeMmioBootLayout {
    pub fn new(
        memory: GuestMemoryRegion,
        entry: GuestPhysAddr,
        stack_pointer: u64,
    ) -> Result<Self, LongModeMmioConfigurationError> {
        Self::with_device_mappings(
            memory,
            entry,
            stack_pointer,
            vec![LongModeMmioPageMapping::new(
                LONG_MODE_MMIO_VIRTUAL_PAGE,
                LONG_MODE_MMIO_DEVICE_GPA,
            )],
        )
    }

    pub fn with_device_mappings(
        memory: GuestMemoryRegion,
        entry: GuestPhysAddr,
        stack_pointer: u64,
        device_mappings: Vec<LongModeMmioPageMapping>,
    ) -> Result<Self, LongModeMmioConfigurationError> {
        let boot = LongModeBootLayout::new(memory, entry, stack_pointer)?;
        validate_device_mappings(memory, &device_mappings)?;
        Ok(Self {
            boot,
            device_mappings,
        })
    }

    #[must_use]
    pub const fn boot_layout(&self) -> &LongModeBootLayout {
        &self.boot
    }

    #[must_use]
    pub fn virtual_page(&self) -> u64 {
        self.device_mappings[0].virtual_page()
    }

    #[must_use]
    pub fn device_gpa(&self) -> u64 {
        self.device_mappings[0].device_gpa()
    }

    #[must_use]
    pub fn device_mappings(&self) -> &[LongModeMmioPageMapping] {
        &self.device_mappings
    }

    pub(crate) fn install_page_tables(&self, memory: &mut GuestMemory) -> Result<(), Error> {
        debug_assert_eq!(memory.region(), self.boot.memory());
        self.boot.install_page_tables(memory)?;
        write_u64(
            memory,
            GuestPhysAddr::new(LONG_MODE_PD_ADDR.get() + ALIAS_PD_INDEX * 8),
            LONG_MODE_ALIAS_PT_ADDR.get() | PAGE_TABLE_ENTRY_FLAGS,
        )?;
        for mapping in &self.device_mappings {
            let pte_index =
                (mapping.virtual_page() - LONG_MODE_ALIAS_VIRTUAL_BASE) / LONG_MODE_PAGE_SIZE;
            write_u64(
                memory,
                GuestPhysAddr::new(LONG_MODE_ALIAS_PT_ADDR.get() + pte_index * 8),
                mapping.device_gpa() | PAGE_TABLE_ENTRY_FLAGS,
            )?;
        }
        Ok(())
    }
}

fn validate_device_mappings(
    memory: GuestMemoryRegion,
    mappings: &[LongModeMmioPageMapping],
) -> Result<(), LongModeMmioConfigurationError> {
    if mappings.is_empty() {
        return Err(LongModeMmioConfigurationError::NoDeviceMappings);
    }

    for (index, mapping) in mappings.iter().enumerate() {
        let virtual_page = mapping.virtual_page();
        let device_page = mapping.device_gpa();
        if virtual_page % LONG_MODE_PAGE_SIZE != 0 {
            return Err(LongModeMmioConfigurationError::VirtualPageMisaligned { virtual_page });
        }
        if !(LONG_MODE_ALIAS_VIRTUAL_BASE..LONG_MODE_ALIAS_VIRTUAL_END).contains(&virtual_page) {
            return Err(LongModeMmioConfigurationError::VirtualPageOutsideAliasWindow {
                virtual_page,
            });
        }
        if device_page % LONG_MODE_PAGE_SIZE != 0 {
            return Err(LongModeMmioConfigurationError::DevicePageMisaligned { device_page });
        }
        device_page.checked_add(LONG_MODE_PAGE_SIZE).ok_or(
            LongModeMmioConfigurationError::DevicePageAddressOverflow { device_page },
        )?;
        if device_page < memory.end().get() {
            return Err(LongModeMmioConfigurationError::DevicePageBackedByRam {
                device_page,
                ram_end: memory.end().get(),
            });
        }
        if mappings[..index]
            .iter()
            .any(|existing| existing.virtual_page() == virtual_page)
        {
            return Err(LongModeMmioConfigurationError::DuplicateVirtualPage { virtual_page });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongModeMmioGuestResult {
    io_exits: Vec<PortIoExit>,
    mmio_exits: Vec<MmioExit>,
    writes: Vec<u8>,
    proof: Vec<u8>,
    report: VmExitReport,
}

impl LongModeMmioGuestResult {
    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn mmio_exits(&self) -> &[MmioExit] {
        &self.mmio_exits
    }

    #[must_use]
    pub fn writes(&self) -> &[u8] {
        &self.writes
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

pub fn run_long_mode_mmio_guest(config: VmConfig) -> Result<LongModeMmioGuestResult, Error> {
    let image = FlatGuestImage::new(
        LONG_MODE_MMIO_GUEST_ENTRY,
        LONG_MODE_MMIO_GUEST_ENTRY,
        &LONG_MODE_MMIO_GUEST_BYTES,
    )?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout =
        LongModeMmioBootLayout::new(memory.region(), image.entry(), LONG_MODE_MMIO_STACK_POINTER)
            .expect("fixed long-mode virtual-MMIO fixture layout remains valid");
    layout.install_page_tables(&mut memory)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode(layout.boot_layout())?;
    let mut port_io = PortIoBus::with_debug_port();
    let mut mmio =
        MmioBus::with_byte_device_at(LONG_MODE_MMIO_DEVICE_GPA, LONG_MODE_MMIO_READ_VALUE);
    let execution = run_vcpu_until_stopped_with_mmio(
        &mut vcpu,
        &mut port_io,
        &mut mmio,
        LONG_MODE_MMIO_EXIT_BUDGET,
    )?;

    if execution.mmio_exits().len() != 2 {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "long-mode virtual MMIO access count",
            expected_reason: crate::vcpu::VcpuExit::Mmio.reason(),
            actual_reason: execution.report().exit().reason(),
        }));
    }

    let writes = mmio.writes().unwrap_or(&[]).to_vec();
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    Ok(LongModeMmioGuestResult {
        io_exits: execution.io_exits().to_vec(),
        mmio_exits: execution.mmio_exits().to_vec(),
        writes,
        proof,
        report: execution.report(),
    })
}

fn write_u64(memory: &mut GuestMemory, address: GuestPhysAddr, value: u64) -> Result<(), Error> {
    memory.write(address, &value.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u64(memory: &GuestMemory, address: GuestPhysAddr) -> u64 {
        let mut bytes = [0_u8; 8];
        memory.read(address, &mut bytes).unwrap();
        u64::from_le_bytes(bytes)
    }

    fn pte_address(virtual_page: u64) -> GuestPhysAddr {
        let pte_index = (virtual_page - LONG_MODE_ALIAS_VIRTUAL_BASE) / LONG_MODE_PAGE_SIZE;
        GuestPhysAddr::new(LONG_MODE_ALIAS_PT_ADDR.get() + pte_index * 8)
    }

    #[test]
    fn long_mode_virtual_mmio_mapping_is_unbacked_and_installed_exactly() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let layout = LongModeMmioBootLayout::new(
            memory.region(),
            LONG_MODE_MMIO_GUEST_ENTRY,
            LONG_MODE_MMIO_STACK_POINTER,
        )
        .unwrap();
        layout.install_page_tables(&mut memory).unwrap();

        assert_eq!(layout.virtual_page(), 0x50_0000);
        assert_eq!(layout.device_gpa(), 0x1000_0000);
        assert_eq!(layout.device_mappings().len(), 1);
        assert!(layout.device_gpa() >= memory.region().end().get());
        assert_eq!(
            read_u64(
                &memory,
                GuestPhysAddr::new(LONG_MODE_PD_ADDR.get() + ALIAS_PD_INDEX * 8)
            ),
            LONG_MODE_ALIAS_PT_ADDR.get() | PAGE_TABLE_ENTRY_FLAGS
        );
        assert_eq!(
            read_u64(&memory, pte_address(LONG_MODE_MMIO_VIRTUAL_PAGE)),
            LONG_MODE_MMIO_DEVICE_GPA | PAGE_TABLE_ENTRY_FLAGS
        );
    }

    #[test]
    fn installs_multiple_distinct_unbacked_device_pages() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let second_virtual = LONG_MODE_MMIO_VIRTUAL_PAGE + LONG_MODE_PAGE_SIZE;
        let second_gpa = LONG_MODE_MMIO_DEVICE_GPA + LONG_MODE_PAGE_SIZE;
        let mappings = vec![
            LongModeMmioPageMapping::new(LONG_MODE_MMIO_VIRTUAL_PAGE, LONG_MODE_MMIO_DEVICE_GPA),
            LongModeMmioPageMapping::new(second_virtual, second_gpa),
        ];
        let layout = LongModeMmioBootLayout::with_device_mappings(
            memory.region(),
            LONG_MODE_MMIO_GUEST_ENTRY,
            LONG_MODE_MMIO_STACK_POINTER,
            mappings.clone(),
        )
        .unwrap();
        layout.install_page_tables(&mut memory).unwrap();

        assert_eq!(layout.device_mappings(), mappings.as_slice());
        assert_eq!(
            read_u64(&memory, pte_address(LONG_MODE_MMIO_VIRTUAL_PAGE)),
            LONG_MODE_MMIO_DEVICE_GPA | PAGE_TABLE_ENTRY_FLAGS
        );
        assert_eq!(
            read_u64(&memory, pte_address(second_virtual)),
            second_gpa | PAGE_TABLE_ENTRY_FLAGS
        );
    }

    #[test]
    fn rejects_invalid_or_duplicate_device_mappings() {
        let memory = GuestMemoryRegion::new(
            GuestPhysAddr::new(0),
            LONG_MODE_IDENTITY_MAP_SIZE,
        )
        .unwrap();
        let valid = LongModeMmioPageMapping::new(
            LONG_MODE_MMIO_VIRTUAL_PAGE,
            LONG_MODE_MMIO_DEVICE_GPA,
        );

        assert!(matches!(
            LongModeMmioBootLayout::with_device_mappings(
                memory,
                LONG_MODE_MMIO_GUEST_ENTRY,
                LONG_MODE_MMIO_STACK_POINTER,
                vec![]
            ),
            Err(LongModeMmioConfigurationError::NoDeviceMappings)
        ));
        assert!(matches!(
            LongModeMmioBootLayout::with_device_mappings(
                memory,
                LONG_MODE_MMIO_GUEST_ENTRY,
                LONG_MODE_MMIO_STACK_POINTER,
                vec![LongModeMmioPageMapping::new(
                    LONG_MODE_MMIO_VIRTUAL_PAGE + 1,
                    LONG_MODE_MMIO_DEVICE_GPA
                )]
            ),
            Err(LongModeMmioConfigurationError::VirtualPageMisaligned { .. })
        ));
        assert!(matches!(
            LongModeMmioBootLayout::with_device_mappings(
                memory,
                LONG_MODE_MMIO_GUEST_ENTRY,
                LONG_MODE_MMIO_STACK_POINTER,
                vec![LongModeMmioPageMapping::new(
                    LONG_MODE_ALIAS_VIRTUAL_END,
                    LONG_MODE_MMIO_DEVICE_GPA
                )]
            ),
            Err(LongModeMmioConfigurationError::VirtualPageOutsideAliasWindow { .. })
        ));
        assert!(matches!(
            LongModeMmioBootLayout::with_device_mappings(
                memory,
                LONG_MODE_MMIO_GUEST_ENTRY,
                LONG_MODE_MMIO_STACK_POINTER,
                vec![LongModeMmioPageMapping::new(
                    LONG_MODE_MMIO_VIRTUAL_PAGE,
                    LONG_MODE_MMIO_DEVICE_GPA + 1
                )]
            ),
            Err(LongModeMmioConfigurationError::DevicePageMisaligned { .. })
        ));
        assert!(matches!(
            LongModeMmioBootLayout::with_device_mappings(
                memory,
                LONG_MODE_MMIO_GUEST_ENTRY,
                LONG_MODE_MMIO_STACK_POINTER,
                vec![valid, valid]
            ),
            Err(LongModeMmioConfigurationError::DuplicateVirtualPage { .. })
        ));
    }

    #[test]
    fn rejects_layout_when_fixed_device_page_would_be_backed_by_ram() {
        let memory = GuestMemoryRegion::new(
            GuestPhysAddr::new(0),
            LONG_MODE_MMIO_DEVICE_GPA + LONG_MODE_PAGE_SIZE,
        )
        .unwrap();
        assert!(matches!(
            LongModeMmioBootLayout::new(
                memory,
                LONG_MODE_MMIO_GUEST_ENTRY,
                LONG_MODE_MMIO_STACK_POINTER
            ),
            Err(LongModeMmioConfigurationError::DevicePageBackedByRam {
                device_page: LONG_MODE_MMIO_DEVICE_GPA,
                ..
            })
        ));
    }

    #[test]
    fn long_mode_virtual_mmio_machine_code_and_terminal_contract_are_stable() {
        assert_eq!(LONG_MODE_MMIO_GUEST_BYTES.len(), 0x1e);
        assert_eq!(LONG_MODE_MMIO_PROOF, b"R64M");
        assert_eq!(LONG_MODE_MMIO_WRITE_VALUE, b'W');
        assert_eq!(LONG_MODE_MMIO_READ_VALUE, b'R');
        assert_eq!(
            LONG_MODE_MMIO_GUEST_ENTRY.get() + LONG_MODE_MMIO_GUEST_BYTES.len() as u64,
            LONG_MODE_MMIO_TERMINAL_RIP
        );
        assert_eq!(
            LONG_MODE_MMIO_GUEST_BYTES,
            [
                0x48, 0xbb, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc6, 0x03, b'W', 0x8a,
                0x03, 0xe6, 0xe9, 0xb0, b'6', 0xe6, 0xe9, 0xb0, b'4', 0xe6, 0xe9, 0xb0, b'M', 0xe6,
                0xe9, 0xf4,
            ]
        );
    }
}
