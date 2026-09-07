use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError, VmExitError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE, LONG_MODE_PML4_ADDR};
use crate::memory::{GuestMemory, GuestMemoryRegion, GuestPhysAddr};
use crate::portio::{PortIoBus, DEBUG_PORT};
use crate::privilege::{
    LongModePrivilegeLayout, PRIVILEGE_GDT_ADDR, PRIVILEGE_IDT_ADDR, PRIVILEGE_KERNEL_ENTRY,
    PRIVILEGE_PT_ADDR, PRIVILEGE_TABLE_END, PRIVILEGE_TERMINAL_HANDLER, PRIVILEGE_TSS_ADDR,
    PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_STACK,
};
use crate::vcpu::{PortIoDirection, PortIoExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::fmt;
use std::io;

pub const ADDRESS_SPACE_A_CR3: GuestPhysAddr = LONG_MODE_PML4_ADDR;
pub const ADDRESS_SPACE_B_PML4_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb000);
pub const ADDRESS_SPACE_B_PDPT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xc000);
pub const ADDRESS_SPACE_B_PD_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xd000);
pub const ADDRESS_SPACE_B_PT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xe000);
pub const ADDRESS_SPACE_CR3_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xf000);
pub const ADDRESS_SPACE_B_USER_CODE_BACKING: GuestPhysAddr = GuestPhysAddr::new(0x20000);
pub const ADDRESS_SPACE_B_USER_DATA_BACKING: GuestPhysAddr = GuestPhysAddr::new(0x21000);
pub const ADDRESS_SPACE_B_USER_STACK_BACKING: GuestPhysAddr = GuestPhysAddr::new(0x22000);
pub const ADDRESS_SPACE_PROOF: &[u8; 4] = b"ABAD";
pub const ADDRESS_SPACE_A_DATA_VALUE: u8 = b'A';
pub const ADDRESS_SPACE_B_DATA_VALUE: u8 = b'B';
pub const ADDRESS_SPACE_A_CONTINUATION_RIP: u64 = 0x1100f;
pub const ADDRESS_SPACE_TERMINAL_RIP: u64 = 0x13031;

const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITABLE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_PAGE_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const ADDRESS_SPACE_EXIT_BUDGET: u32 = 5;

const KERNEL_BOOT_BYTES: [u8; 41] = [
    0xfa, 0x66, 0xb8, 0x28, 0x00, 0x0f, 0x00, 0xd8, 0x6a, 0x1b, 0x48, 0xb8, 0x00, 0xd0, 0x1f, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x50, 0x68, 0x02, 0x02, 0x00, 0x00, 0x6a, 0x23, 0x48, 0xb8, 0x00, 0x10,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x50, 0x48, 0xcf,
];

const TASK_A_BYTES: [u8; 17] = [
    0x48, 0xbf, 0x00, 0xa0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc6, 0x07, b'A', 0xcd, 0x80, 0xcd,
    0x81,
];

const TASK_B_BYTES: [u8; 17] = [
    0x48, 0xbf, 0x00, 0xa0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc6, 0x07, b'B', 0xcd, 0x80, 0xcd,
    0x81,
];

const SWITCH_HANDLER_BYTES: [u8; 172] = [
    15, 32, 219, 72, 191, 0, 160, 0, 0, 0, 0, 0, 0, 138, 7, 230, 233, 72, 129, 251, 0, 16, 0, 0,
    15, 132, 18, 0, 0, 0, 72, 129, 251, 0, 176, 0, 0, 15, 132, 64, 0, 0, 0, 233, 119, 0, 0, 0, 72,
    191, 0, 240, 0, 0, 0, 0, 0, 0, 72, 137, 31, 72, 184, 0, 176, 0, 0, 0, 0, 0, 0, 15, 34, 216,
    106, 27, 72, 184, 0, 208, 31, 0, 0, 0, 0, 0, 80, 104, 2, 2, 0, 0, 106, 35, 72, 184, 0, 16, 1,
    0, 0, 0, 0, 0, 80, 72, 207, 72, 191, 0, 240, 0, 0, 0, 0, 0, 0, 72, 137, 95, 8, 72, 184, 0, 16,
    0, 0, 0, 0, 0, 0, 15, 34, 216, 106, 27, 72, 184, 0, 208, 31, 0, 0, 0, 0, 0, 80, 104, 2, 2, 0,
    0, 106, 35, 72, 184, 15, 16, 1, 0, 0, 0, 0, 0, 80, 72, 207, 176, 70, 230, 233, 244,
];

const TERMINAL_HANDLER_BYTES: [u8; 54] = [
    15, 32, 219, 72, 129, 251, 0, 16, 0, 0, 15, 133, 33, 0, 0, 0, 72, 191, 0, 240, 0, 0, 0, 0, 0,
    0, 72, 137, 95, 16, 72, 191, 0, 160, 0, 0, 0, 0, 0, 0, 138, 7, 230, 233, 176, 68, 230, 233,
    244, 176, 70, 230, 233, 244,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressSpaceConfigurationError {
    Privilege(crate::privilege::PrivilegeConfigurationError),
    AddressOutsideMemory {
        role: &'static str,
        address: u64,
    },
    MisalignedAddress {
        role: &'static str,
        address: u64,
    },
    DuplicateReservedPage {
        first: &'static str,
        second: &'static str,
        address: u64,
    },
}

impl fmt::Display for AddressSpaceConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Privilege(error) => error.fmt(f),
            Self::AddressOutsideMemory { role, address } => {
                write!(f, "{role} address {address:#x} is outside registered RAM")
            }
            Self::MisalignedAddress { role, address } => {
                write!(f, "{role} address {address:#x} is not 4 KiB aligned")
            }
            Self::DuplicateReservedPage {
                first,
                second,
                address,
            } => write!(
                f,
                "{first} and {second} both reserve physical page {address:#x}"
            ),
        }
    }
}

impl std::error::Error for AddressSpaceConfigurationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Privilege(error) => Some(error),
            _ => None,
        }
    }
}

impl From<crate::privilege::PrivilegeConfigurationError> for AddressSpaceConfigurationError {
    fn from(error: crate::privilege::PrivilegeConfigurationError) -> Self {
        Self::Privilege(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressSpaceSwitchLayout {
    privilege: LongModePrivilegeLayout,
    memory: GuestMemoryRegion,
}

impl AddressSpaceSwitchLayout {
    pub fn new(memory: GuestMemoryRegion) -> Result<Self, AddressSpaceConfigurationError> {
        let privilege = LongModePrivilegeLayout::new(memory)?;
        let reserved = [
            ("B PML4", ADDRESS_SPACE_B_PML4_ADDR.get()),
            ("B PDPT", ADDRESS_SPACE_B_PDPT_ADDR.get()),
            ("B PD", ADDRESS_SPACE_B_PD_ADDR.get()),
            ("B PT", ADDRESS_SPACE_B_PT_ADDR.get()),
            ("CR3 observation", ADDRESS_SPACE_CR3_OBSERVATION_ADDR.get()),
            (
                "B user code backing",
                ADDRESS_SPACE_B_USER_CODE_BACKING.get(),
            ),
            (
                "B user data backing",
                ADDRESS_SPACE_B_USER_DATA_BACKING.get(),
            ),
            (
                "B user stack backing",
                ADDRESS_SPACE_B_USER_STACK_BACKING.get(),
            ),
        ];
        for (role, address) in reserved {
            if address % LONG_MODE_PAGE_SIZE != 0 {
                return Err(AddressSpaceConfigurationError::MisalignedAddress { role, address });
            }
            if address < memory.base().get() || address + LONG_MODE_PAGE_SIZE > memory.end().get() {
                return Err(AddressSpaceConfigurationError::AddressOutsideMemory { role, address });
            }
        }
        for (index, (first, first_address)) in reserved.iter().enumerate() {
            for (second, second_address) in reserved.iter().skip(index + 1) {
                if first_address == second_address {
                    return Err(AddressSpaceConfigurationError::DuplicateReservedPage {
                        first,
                        second,
                        address: *first_address,
                    });
                }
            }
        }
        Ok(Self { privilege, memory })
    }

    #[must_use]
    pub const fn privilege_layout(&self) -> &LongModePrivilegeLayout {
        &self.privilege
    }

    #[must_use]
    pub const fn first_cr3(&self) -> GuestPhysAddr {
        ADDRESS_SPACE_A_CR3
    }

    #[must_use]
    pub const fn second_cr3(&self) -> GuestPhysAddr {
        ADDRESS_SPACE_B_PML4_ADDR
    }

    pub(crate) fn install_tables(&self, memory: &mut GuestMemory) -> Result<(), Error> {
        debug_assert_eq!(memory.region(), self.memory);
        self.privilege.install_tables(memory)?;
        let zero_page = [0_u8; LONG_MODE_PAGE_SIZE as usize];
        for page in [
            ADDRESS_SPACE_B_PML4_ADDR,
            ADDRESS_SPACE_B_PDPT_ADDR,
            ADDRESS_SPACE_B_PD_ADDR,
            ADDRESS_SPACE_B_PT_ADDR,
            ADDRESS_SPACE_CR3_OBSERVATION_ADDR,
        ] {
            memory.write(page, &zero_page)?;
        }
        install_second_root(memory)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressSpaceSwitchGuestResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
    cr3_observations: [u64; 3],
    final_cr3: u64,
    first_data: u8,
    second_data: u8,
    first_code_pte: u64,
    second_code_pte: u64,
    first_data_pte: u64,
    second_data_pte: u64,
    first_stack_pte: u64,
    second_stack_pte: u64,
    first_kernel_pte: u64,
    second_kernel_pte: u64,
}

impl AddressSpaceSwitchGuestResult {
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
    pub const fn cr3_observations(&self) -> [u64; 3] {
        self.cr3_observations
    }

    #[must_use]
    pub const fn final_cr3(&self) -> u64 {
        self.final_cr3
    }

    #[must_use]
    pub const fn first_data(&self) -> u8 {
        self.first_data
    }

    #[must_use]
    pub const fn second_data(&self) -> u8 {
        self.second_data
    }

    #[must_use]
    pub const fn first_code_pte(&self) -> u64 {
        self.first_code_pte
    }

    #[must_use]
    pub const fn second_code_pte(&self) -> u64 {
        self.second_code_pte
    }

    #[must_use]
    pub const fn first_data_pte(&self) -> u64 {
        self.first_data_pte
    }

    #[must_use]
    pub const fn second_data_pte(&self) -> u64 {
        self.second_data_pte
    }

    #[must_use]
    pub const fn first_stack_pte(&self) -> u64 {
        self.first_stack_pte
    }

    #[must_use]
    pub const fn second_stack_pte(&self) -> u64 {
        self.second_stack_pte
    }

    #[must_use]
    pub const fn first_kernel_pte(&self) -> u64 {
        self.first_kernel_pte
    }

    #[must_use]
    pub const fn second_kernel_pte(&self) -> u64 {
        self.second_kernel_pte
    }
}

pub fn run_address_space_switch_guest(
    config: VmConfig,
) -> Result<AddressSpaceSwitchGuestResult, Error> {
    let kernel = FlatGuestImage::new(
        PRIVILEGE_KERNEL_ENTRY,
        PRIVILEGE_KERNEL_ENTRY,
        &KERNEL_BOOT_BYTES,
    )?;
    let first_user =
        FlatGuestImage::new(PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_ENTRY, &TASK_A_BYTES)?;
    let second_user = FlatGuestImage::new(
        ADDRESS_SPACE_B_USER_CODE_BACKING,
        ADDRESS_SPACE_B_USER_CODE_BACKING,
        &TASK_B_BYTES,
    )?;
    let switch_handler = FlatGuestImage::new(
        crate::privilege::PRIVILEGE_RETURN_HANDLER,
        crate::privilege::PRIVILEGE_RETURN_HANDLER,
        &SWITCH_HANDLER_BYTES,
    )?;
    let terminal_handler = FlatGuestImage::new(
        PRIVILEGE_TERMINAL_HANDLER,
        PRIVILEGE_TERMINAL_HANDLER,
        &TERMINAL_HANDLER_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = AddressSpaceSwitchLayout::new(memory.region())
        .expect("fixed bounded two-address-space layout remains valid");
    layout.install_tables(&mut memory)?;
    kernel.load(&mut memory)?;
    first_user.load(&mut memory)?;
    second_user.load(&mut memory)?;
    switch_handler.load(&mut memory)?;
    terminal_handler.load(&mut memory)?;
    memory.write(GuestPhysAddr::new(0xa000), &[0])?;
    memory.write(ADDRESS_SPACE_B_USER_DATA_BACKING, &[0])?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(layout.privilege_layout())?;
    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, ADDRESS_SPACE_EXIT_BUDGET)?;
    if execution.io_exits().len() != ADDRESS_SPACE_PROOF.len() {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "address-space switch proof output count",
            expected_reason: crate::vcpu::VcpuExit::Io.reason(),
            actual_reason: execution.report().exit().reason(),
        }));
    }
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != ADDRESS_SPACE_PROOF {
        return Err(verification_error(
            "address-space switch proof",
            format!("expected {ADDRESS_SPACE_PROOF:?}, got {proof:?}"),
        ));
    }
    for (io, expected) in execution
        .io_exits()
        .iter()
        .zip(ADDRESS_SPACE_PROOF.iter().copied())
    {
        if io.direction() != PortIoDirection::Out
            || io.port() != DEBUG_PORT
            || io.size() != 1
            || io.count() != 1
            || io.output_data() != [expected]
        {
            return Err(verification_error(
                "address-space switch debug-port exit",
                format!("unexpected I/O exit {io:?}, expected byte {expected:#x}"),
            ));
        }
    }

    let special = vcpu.capture_special_register_snapshot()?;
    let guest_memory = vm
        .guest_memory()
        .expect("registered address-space memory remains VM-owned");
    let cr3_observations = read_cr3_observations(guest_memory)?;
    let first_data = read_byte(guest_memory, GuestPhysAddr::new(0xa000))?;
    let second_data = read_byte(guest_memory, ADDRESS_SPACE_B_USER_DATA_BACKING)?;
    let first_code_pte = read_pte(guest_memory, PRIVILEGE_PT_ADDR, PRIVILEGE_USER_ENTRY.get())?;
    let second_code_pte = read_pte(
        guest_memory,
        ADDRESS_SPACE_B_PT_ADDR,
        PRIVILEGE_USER_ENTRY.get(),
    )?;
    let first_data_pte = read_pte(guest_memory, PRIVILEGE_PT_ADDR, 0xa000)?;
    let second_data_pte = read_pte(guest_memory, ADDRESS_SPACE_B_PT_ADDR, 0xa000)?;
    let first_stack_pte = read_pte(guest_memory, PRIVILEGE_PT_ADDR, PRIVILEGE_USER_STACK - 1)?;
    let second_stack_pte = read_pte(
        guest_memory,
        ADDRESS_SPACE_B_PT_ADDR,
        PRIVILEGE_USER_STACK - 1,
    )?;
    let first_kernel_pte = read_pte(
        guest_memory,
        PRIVILEGE_PT_ADDR,
        crate::privilege::PRIVILEGE_RETURN_HANDLER.get(),
    )?;
    let second_kernel_pte = read_pte(
        guest_memory,
        ADDRESS_SPACE_B_PT_ADDR,
        crate::privilege::PRIVILEGE_RETURN_HANDLER.get(),
    )?;

    validate_result(
        execution.report(),
        cr3_observations,
        special.cr3(),
        first_data,
        second_data,
        first_code_pte,
        second_code_pte,
        first_data_pte,
        second_data_pte,
        first_stack_pte,
        second_stack_pte,
        first_kernel_pte,
        second_kernel_pte,
    )?;

    Ok(AddressSpaceSwitchGuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
        cr3_observations,
        final_cr3: special.cr3(),
        first_data,
        second_data,
        first_code_pte,
        second_code_pte,
        first_data_pte,
        second_data_pte,
        first_stack_pte,
        second_stack_pte,
        first_kernel_pte,
        second_kernel_pte,
    })
}

fn install_second_root(memory: &mut GuestMemory) -> Result<(), Error> {
    write_u64(
        memory,
        ADDRESS_SPACE_B_PML4_ADDR,
        ADDRESS_SPACE_B_PDPT_ADDR.get() | 0x7,
    )?;
    write_u64(
        memory,
        ADDRESS_SPACE_B_PDPT_ADDR,
        ADDRESS_SPACE_B_PD_ADDR.get() | 0x7,
    )?;
    write_u64(
        memory,
        ADDRESS_SPACE_B_PD_ADDR,
        ADDRESS_SPACE_B_PT_ADDR.get() | 0x7,
    )?;
    for index in 0..512_u64 {
        let virtual_address = index * LONG_MODE_PAGE_SIZE;
        let physical_address = match virtual_address {
            0xa000 => ADDRESS_SPACE_B_USER_DATA_BACKING.get(),
            address if address == PRIVILEGE_USER_ENTRY.get() => {
                ADDRESS_SPACE_B_USER_CODE_BACKING.get()
            }
            address if address == page_start(PRIVILEGE_USER_STACK - 1) => {
                ADDRESS_SPACE_B_USER_STACK_BACKING.get()
            }
            address => address,
        };
        let flags = X86_PAGE_PRESENT
            | X86_PAGE_WRITABLE
            | if is_user_virtual_page(virtual_address) {
                X86_PAGE_USER
            } else {
                0
            };
        write_u64(
            memory,
            GuestPhysAddr::new(ADDRESS_SPACE_B_PT_ADDR.get() + index * 8),
            physical_address | flags,
        )?;
    }
    Ok(())
}

fn is_user_virtual_page(address: u64) -> bool {
    let page = page_start(address);
    page == page_start(PRIVILEGE_USER_ENTRY.get())
        || page == 0xa000
        || page == page_start(PRIVILEGE_USER_STACK - 1)
}

const fn page_start(address: u64) -> u64 {
    address & !(LONG_MODE_PAGE_SIZE - 1)
}

fn read_cr3_observations(memory: &GuestMemory) -> Result<[u64; 3], Error> {
    let mut bytes = [0_u8; 24];
    memory.read(ADDRESS_SPACE_CR3_OBSERVATION_ADDR, &mut bytes)?;
    Ok([
        u64::from_le_bytes(bytes[0..8].try_into().expect("first CR3 field")),
        u64::from_le_bytes(bytes[8..16].try_into().expect("second CR3 field")),
        u64::from_le_bytes(bytes[16..24].try_into().expect("terminal CR3 field")),
    ])
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
    let index = page_start(virtual_address) / LONG_MODE_PAGE_SIZE;
    let mut bytes = [0_u8; 8];
    memory.read(GuestPhysAddr::new(table.get() + index * 8), &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn write_u64(memory: &mut GuestMemory, address: GuestPhysAddr, value: u64) -> Result<(), Error> {
    memory.write(address, &value.to_le_bytes())
}

#[allow(clippy::too_many_arguments)]
fn validate_result(
    report: VmExitReport,
    cr3_observations: [u64; 3],
    final_cr3: u64,
    first_data: u8,
    second_data: u8,
    first_code_pte: u64,
    second_code_pte: u64,
    first_data_pte: u64,
    second_data_pte: u64,
    first_stack_pte: u64,
    second_stack_pte: u64,
    first_kernel_pte: u64,
    second_kernel_pte: u64,
) -> Result<(), Error> {
    if report.exit() != crate::vcpu::VcpuExit::Hlt || report.rip() != ADDRESS_SPACE_TERMINAL_RIP {
        return Err(verification_error(
            "address-space terminal exit",
            format!("expected HLT at RIP {ADDRESS_SPACE_TERMINAL_RIP:#x}, got {report}"),
        ));
    }
    let expected_cr3 = [
        ADDRESS_SPACE_A_CR3.get(),
        ADDRESS_SPACE_B_PML4_ADDR.get(),
        ADDRESS_SPACE_A_CR3.get(),
    ];
    if cr3_observations != expected_cr3 || final_cr3 != ADDRESS_SPACE_A_CR3.get() {
        return Err(verification_error(
            "address-space CR3 ownership",
            format!(
                "expected observations {:?} and final {:#x}, got {:?} and {final_cr3:#x}",
                expected_cr3,
                ADDRESS_SPACE_A_CR3.get(),
                cr3_observations
            ),
        ));
    }
    if first_data != ADDRESS_SPACE_A_DATA_VALUE || second_data != ADDRESS_SPACE_B_DATA_VALUE {
        return Err(verification_error(
            "address-space physical data isolation",
            format!(
                "expected A/B data {:?}/{:?}, got {first_data:?}/{second_data:?}",
                ADDRESS_SPACE_A_DATA_VALUE, ADDRESS_SPACE_B_DATA_VALUE
            ),
        ));
    }
    for (role, pte, expected_physical, user) in [
        ("A code", first_code_pte, PRIVILEGE_USER_ENTRY.get(), true),
        (
            "B code",
            second_code_pte,
            ADDRESS_SPACE_B_USER_CODE_BACKING.get(),
            true,
        ),
        ("A data", first_data_pte, 0xa000, true),
        (
            "B data",
            second_data_pte,
            ADDRESS_SPACE_B_USER_DATA_BACKING.get(),
            true,
        ),
        (
            "A stack",
            first_stack_pte,
            page_start(PRIVILEGE_USER_STACK - 1),
            true,
        ),
        (
            "B stack",
            second_stack_pte,
            ADDRESS_SPACE_B_USER_STACK_BACKING.get(),
            true,
        ),
        (
            "A switch handler",
            first_kernel_pte,
            crate::privilege::PRIVILEGE_RETURN_HANDLER.get(),
            false,
        ),
        (
            "B switch handler",
            second_kernel_pte,
            crate::privilege::PRIVILEGE_RETURN_HANDLER.get(),
            false,
        ),
    ] {
        validate_pte(role, pte, expected_physical, user)?;
    }
    Ok(())
}

fn validate_pte(role: &'static str, pte: u64, physical: u64, user: bool) -> Result<(), Error> {
    let required = X86_PAGE_PRESENT | X86_PAGE_WRITABLE | if user { X86_PAGE_USER } else { 0 };
    if pte & X86_PAGE_ADDRESS_MASK != physical
        || pte & required != required
        || (!user && pte & X86_PAGE_USER != 0)
    {
        return Err(verification_error(
            "address-space PTE ownership",
            format!("{role}: expected physical {physical:#x}, user={user}, got PTE {pte:#x}"),
        ));
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
    assert!(ADDRESS_SPACE_B_PML4_ADDR.get() >= PRIVILEGE_TABLE_END.get());
    assert!(ADDRESS_SPACE_B_USER_STACK_BACKING.get() < LONG_MODE_IDENTITY_MAP_SIZE);
    assert!(PRIVILEGE_GDT_ADDR.get() < PRIVILEGE_IDT_ADDR.get());
    assert!(PRIVILEGE_TSS_ADDR.get() < ADDRESS_SPACE_B_PML4_ADDR.get());
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_second_root_maps_same_user_virtual_pages_to_distinct_backing() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let layout = AddressSpaceSwitchLayout::new(memory.region()).unwrap();
        layout.install_tables(&mut memory).unwrap();

        let a_code = read_pte(&memory, PRIVILEGE_PT_ADDR, PRIVILEGE_USER_ENTRY.get()).unwrap();
        let b_code =
            read_pte(&memory, ADDRESS_SPACE_B_PT_ADDR, PRIVILEGE_USER_ENTRY.get()).unwrap();
        let a_data = read_pte(&memory, PRIVILEGE_PT_ADDR, 0xa000).unwrap();
        let b_data = read_pte(&memory, ADDRESS_SPACE_B_PT_ADDR, 0xa000).unwrap();
        let a_stack = read_pte(&memory, PRIVILEGE_PT_ADDR, PRIVILEGE_USER_STACK - 1).unwrap();
        let b_stack = read_pte(&memory, ADDRESS_SPACE_B_PT_ADDR, PRIVILEGE_USER_STACK - 1).unwrap();
        let a_kernel = read_pte(
            &memory,
            PRIVILEGE_PT_ADDR,
            crate::privilege::PRIVILEGE_RETURN_HANDLER.get(),
        )
        .unwrap();
        let b_kernel = read_pte(
            &memory,
            ADDRESS_SPACE_B_PT_ADDR,
            crate::privilege::PRIVILEGE_RETURN_HANDLER.get(),
        )
        .unwrap();

        validate_pte("A code", a_code, PRIVILEGE_USER_ENTRY.get(), true).unwrap();
        validate_pte(
            "B code",
            b_code,
            ADDRESS_SPACE_B_USER_CODE_BACKING.get(),
            true,
        )
        .unwrap();
        validate_pte("A data", a_data, 0xa000, true).unwrap();
        validate_pte(
            "B data",
            b_data,
            ADDRESS_SPACE_B_USER_DATA_BACKING.get(),
            true,
        )
        .unwrap();
        validate_pte(
            "A stack",
            a_stack,
            page_start(PRIVILEGE_USER_STACK - 1),
            true,
        )
        .unwrap();
        validate_pte(
            "B stack",
            b_stack,
            ADDRESS_SPACE_B_USER_STACK_BACKING.get(),
            true,
        )
        .unwrap();
        validate_pte(
            "A handler",
            a_kernel,
            crate::privilege::PRIVILEGE_RETURN_HANDLER.get(),
            false,
        )
        .unwrap();
        validate_pte(
            "B handler",
            b_kernel,
            crate::privilege::PRIVILEGE_RETURN_HANDLER.get(),
            false,
        )
        .unwrap();
    }

    #[test]
    fn switch_and_terminal_machine_code_keep_exact_cr3_and_continuation_contract() {
        assert_eq!(SWITCH_HANDLER_BYTES.len(), 172);
        assert_eq!(TERMINAL_HANDLER_BYTES.len(), 54);
        assert_eq!(&TASK_A_BYTES[13..17], &[0xcd, 0x80, 0xcd, 0x81]);
        assert_eq!(
            PRIVILEGE_USER_ENTRY.get() + 15,
            ADDRESS_SPACE_A_CONTINUATION_RIP
        );
        assert_eq!(TERMINAL_HANDLER_BYTES[48], 0xf4);
    }
}
