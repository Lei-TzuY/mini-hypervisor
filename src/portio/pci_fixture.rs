use super::pci::{
    config_selector, PciConfigMechanism1, SyntheticPciFunction, PCI_CONFIG_ADDRESS_PORT,
    PCI_CONFIG_DATA_PORT,
};
#[cfg(test)]
use super::pci::{
    SYNTHETIC_PCI_CLASS_CODE, SYNTHETIC_PCI_DEVICE_ID, SYNTHETIC_PCI_REVISION,
    SYNTHETIC_PCI_VENDOR_ID,
};
use super::{PortIoBus, DEBUG_PORT};
use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError};
use crate::execution::run_vcpu_until_stopped_with_mmio;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::mmio::long_mode::{
    LongModeMmioBootLayout, LONG_MODE_MMIO_DEVICE_GPA, LONG_MODE_MMIO_GUEST_ENTRY,
    LONG_MODE_MMIO_STACK_POINTER,
};
use crate::mmio::MmioBus;
use crate::vcpu::{MmioDirection, MmioExit, PortIoDirection, PortIoExit, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;

pub const PCI_BAR0_GPA: u64 = LONG_MODE_MMIO_DEVICE_GPA;
pub const PCI_BAR_WRITE_VALUE: u8 = b'W';
pub const PCI_DISCOVERY_PROOF: &[u8; 4] = b"PCBM";
pub const PCI_DISCOVERY_TERMINAL_RIP: u64 = LONG_MODE_MMIO_GUEST_ENTRY.get() + 101;

const PCI_DISCOVERY_EXIT_BUDGET: u32 = 12;
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;

const PCI_DISCOVERY_GUEST_BYTES: [u8; 106] = [
    0x66,
    0xba,
    0xf8,
    0x0c, // mov $0xcf8, %dx
    0xb8,
    0x00,
    0x08,
    0x00,
    0x80, // mov $0x80000800, %eax -- 00:01.0 identity
    0xef, // out %eax, %dx
    0x66,
    0xba,
    0xfc,
    0x0c, // mov $0xcfc, %dx
    0xed, // in %dx, %eax
    0x3d,
    0xfe,
    0xca,
    0x01,
    0x00, // cmp $0x0001cafe, %eax
    0x75,
    0x4f, // jne failure
    0xb0,
    b'P',
    0xe6,
    0xe9, // proof: PCI identity
    0x66,
    0xba,
    0xf8,
    0x0c,
    0xb8,
    0x08,
    0x08,
    0x00,
    0x80, // select class/revision dword
    0xef,
    0x66,
    0xba,
    0xfc,
    0x0c,
    0xed,
    0x3d,
    0x01,
    0x00,
    0x00,
    0xff, // class 0xff, revision 1
    0x75,
    0x35, // jne failure
    0xb0,
    b'C',
    0xe6,
    0xe9, // proof: class/revision
    0x66,
    0xba,
    0xf8,
    0x0c,
    0xb8,
    0x10,
    0x08,
    0x00,
    0x80, // select BAR0
    0xef,
    0x66,
    0xba,
    0xfc,
    0x0c,
    0xed,
    0x25,
    0xf0,
    0xff,
    0xff,
    0xff, // mask BAR attribute bits
    0x3d,
    0x00,
    0x00,
    0x00,
    0x10, // cmp $0x10000000, %eax
    0x75,
    0x16, // jne failure
    0xb0,
    b'B',
    0xe6,
    0xe9, // proof: BAR0
    0x48,
    0xbb,
    0x00,
    0x00,
    0x50,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00, // movabs $0x500000, %rbx
    0xc6,
    0x03,
    PCI_BAR_WRITE_VALUE, // write W through VA mapped to BAR0 GPA
    0xb0,
    b'M',
    0xe6,
    0xe9, // proof: MMIO completed
    0xf4, // hlt
    0xb0,
    b'F',
    0xe6,
    0xe9,
    0xf4, // failure proof + hlt
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PciDiscoveryGuestResult {
    io_exits: Vec<PortIoExit>,
    mmio_exits: Vec<MmioExit>,
    writes: Vec<u8>,
    proof: Vec<u8>,
    report: VmExitReport,
}

impl PciDiscoveryGuestResult {
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

pub fn run_pci_discovery_guest(config: VmConfig) -> Result<PciDiscoveryGuestResult, Error> {
    let image = FlatGuestImage::new(
        LONG_MODE_MMIO_GUEST_ENTRY,
        LONG_MODE_MMIO_GUEST_ENTRY,
        &PCI_DISCOVERY_GUEST_BYTES,
    )?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout =
        LongModeMmioBootLayout::new(memory.region(), image.entry(), LONG_MODE_MMIO_STACK_POINTER)
            .expect("fixed PCI discovery MMIO mapping remains valid");
    layout.install_page_tables(&mut memory)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode(layout.boot_layout())?;

    let pci = PciConfigMechanism1::new(SyntheticPciFunction::new(PCI_BAR0_GPA as u32));
    let mut port_io = PortIoBus::with_debug_port_and_pci_config(pci);
    let mut mmio = MmioBus::empty();
    mmio.register_byte_device_at(PCI_BAR0_GPA, 0)
        .expect("fixed PCI BAR0 MMIO device remains non-overlapping");

    let execution = run_vcpu_until_stopped_with_mmio(
        &mut vcpu,
        &mut port_io,
        &mut mmio,
        PCI_DISCOVERY_EXIT_BUDGET,
    )?;
    validate_io_sequence(execution.io_exits())?;
    validate_mmio(execution.mmio_exits())?;

    let writes = mmio.writes_at(PCI_BAR0_GPA).unwrap_or(&[]).to_vec();
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    let report = execution.report();

    if writes.as_slice() != [PCI_BAR_WRITE_VALUE] || proof.as_slice() != PCI_DISCOVERY_PROOF {
        return Err(verification_error(format!(
            "expected BAR writes {:?} and proof {:?}; got writes {:?}, proof {:?}",
            [PCI_BAR_WRITE_VALUE],
            PCI_DISCOVERY_PROOF,
            writes,
            proof
        )));
    }

    if report.exit() != VcpuExit::Hlt
        || report.rip() != PCI_DISCOVERY_TERMINAL_RIP
        || report.rflags() & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
    {
        return Err(verification_error(format!(
            "expected HLT at RIP {PCI_DISCOVERY_TERMINAL_RIP:#x} with architectural RFLAGS bit 1 set, got {report}"
        )));
    }

    Ok(PciDiscoveryGuestResult {
        io_exits: execution.io_exits().to_vec(),
        mmio_exits: execution.mmio_exits().to_vec(),
        writes,
        proof,
        report,
    })
}

fn validate_io_sequence(exits: &[PortIoExit]) -> Result<(), Error> {
    let selectors = [
        config_selector(0x00),
        config_selector(0x08),
        config_selector(0x10),
    ];
    let expected_proof = *b"PCBM";
    if exits.len() != 10 {
        return Err(verification_error(format!(
            "expected 10 port-I/O exits, got {}",
            exits.len()
        )));
    }

    for (cycle, selector) in selectors.into_iter().enumerate() {
        let base = cycle * 3;
        let address = &exits[base];
        let data = &exits[base + 1];
        let proof = &exits[base + 2];
        if address.direction() != PortIoDirection::Out
            || address.port() != PCI_CONFIG_ADDRESS_PORT
            || address.size() != 4
            || address.count() != 1
            || address.output_data() != selector.to_le_bytes()
            || data.direction() != PortIoDirection::In
            || data.port() != PCI_CONFIG_DATA_PORT
            || data.size() != 4
            || data.count() != 1
            || !data.output_data().is_empty()
            || proof.direction() != PortIoDirection::Out
            || proof.port() != DEBUG_PORT
            || proof.size() != 1
            || proof.count() != 1
            || proof.output_data() != [expected_proof[cycle]]
        {
            return Err(verification_error(format!(
                "PCI config cycle {cycle} did not match selector {selector:#010x} and proof byte {:?}",
                expected_proof[cycle]
            )));
        }
    }

    let completion = &exits[9];
    if completion.direction() != PortIoDirection::Out
        || completion.port() != DEBUG_PORT
        || completion.size() != 1
        || completion.count() != 1
        || completion.output_data() != [expected_proof[3]]
    {
        return Err(verification_error(
            "PCI BAR MMIO completion output did not match M",
        ));
    }
    Ok(())
}

fn validate_mmio(exits: &[MmioExit]) -> Result<(), Error> {
    if exits.len() != 1 {
        return Err(verification_error(format!(
            "expected one BAR-backed MMIO exit, got {}",
            exits.len()
        )));
    }
    let exit = &exits[0];
    if exit.address() != PCI_BAR0_GPA
        || exit.direction() != MmioDirection::Write
        || exit.length() != 1
        || exit.write_data() != [PCI_BAR_WRITE_VALUE]
    {
        return Err(verification_error(format!(
            "expected BAR0 write at {PCI_BAR0_GPA:#x} with {:?}, got address {:#x}, direction {:?}, length {}, data {:?}",
            [PCI_BAR_WRITE_VALUE],
            exit.address(),
            exit.direction(),
            exit.length(),
            exit.write_data()
        )));
    }
    Ok(())
}

fn verification_error(detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation: "PCI configuration BAR discovery proof",
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_machine_code_and_terminal_rip_are_stable() {
        assert_eq!(PCI_DISCOVERY_GUEST_BYTES.len(), 106);
        assert_eq!(
            PCI_DISCOVERY_TERMINAL_RIP,
            LONG_MODE_MMIO_GUEST_ENTRY.get() + 101
        );
        assert_eq!(PCI_DISCOVERY_GUEST_BYTES[100], 0xf4);
        assert_eq!(
            &PCI_DISCOVERY_GUEST_BYTES[101..],
            &[0xb0, b'F', 0xe6, 0xe9, 0xf4]
        );
    }

    #[test]
    fn synthetic_identity_constants_are_not_standard_virtio_claims() {
        assert_eq!(SYNTHETIC_PCI_VENDOR_ID, 0xcafe);
        assert_eq!(SYNTHETIC_PCI_DEVICE_ID, 0x0001);
        assert_eq!(SYNTHETIC_PCI_CLASS_CODE, 0xff);
        assert_eq!(SYNTHETIC_PCI_REVISION, 1);
    }
}
