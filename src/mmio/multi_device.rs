mod virtio_blk_atomic_bus;

use super::long_mode::{
    LongModeMmioBootLayout, LongModeMmioPageMapping, LONG_MODE_MMIO_DEVICE_GPA,
    LONG_MODE_MMIO_GUEST_ENTRY, LONG_MODE_MMIO_STACK_POINTER, LONG_MODE_MMIO_VIRTUAL_PAGE,
};
use super::MmioBus;
use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError};
use crate::execution::run_vcpu_until_stopped_with_mmio;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::PortIoBus;
use crate::vcpu::{MmioDirection, MmioExit, PortIoExit, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;

pub const MULTI_DEVICE_SECOND_VIRTUAL_PAGE: u64 = LONG_MODE_MMIO_VIRTUAL_PAGE + LONG_MODE_PAGE_SIZE;
pub const MULTI_DEVICE_SECOND_GPA: u64 = LONG_MODE_MMIO_DEVICE_GPA + LONG_MODE_PAGE_SIZE;
pub const MULTI_DEVICE_FIRST_READ_VALUE: u8 = b'A';
pub const MULTI_DEVICE_SECOND_READ_VALUE: u8 = b'B';
pub const MULTI_DEVICE_FIRST_WRITE_VALUE: u8 = b'X';
pub const MULTI_DEVICE_SECOND_WRITE_VALUE: u8 = b'Y';
pub const MULTI_DEVICE_PROOF: &[u8; 4] = b"ABAM";
pub const MULTI_DEVICE_TERMINAL_RIP: u64 = LONG_MODE_MMIO_GUEST_ENTRY.get() + 43;

const MULTI_DEVICE_EXIT_BUDGET: u32 = 10;
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;

const MULTI_DEVICE_GUEST_BYTES: [u8; 43] = [
    0x48,
    0xbb,
    0x00,
    0x00,
    0x50,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00, // movabs $0x500000, %rbx -- first device VA
    0x48,
    0xb9,
    0x00,
    0x10,
    0x50,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00, // movabs $0x501000, %rcx -- second device VA
    0xc6,
    0x03,
    MULTI_DEVICE_FIRST_WRITE_VALUE, // first write X
    0x8a,
    0x03, // first read -> A
    0xe6,
    0xe9, // output A
    0xc6,
    0x01,
    MULTI_DEVICE_SECOND_WRITE_VALUE, // second write Y
    0x8a,
    0x01, // second read -> B
    0xe6,
    0xe9, // output B
    0x8a,
    0x03, // first read again -> A
    0xe6,
    0xe9, // output A, proving first device remained independently routed
    0xb0,
    b'M',
    0xe6,
    0xe9, // completion proof
    0xf4, // hlt
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiDeviceMmioGuestResult {
    io_exits: Vec<PortIoExit>,
    mmio_exits: Vec<MmioExit>,
    first_writes: Vec<u8>,
    second_writes: Vec<u8>,
    proof: Vec<u8>,
    report: VmExitReport,
}

impl MultiDeviceMmioGuestResult {
    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn mmio_exits(&self) -> &[MmioExit] {
        &self.mmio_exits
    }

    #[must_use]
    pub fn first_writes(&self) -> &[u8] {
        &self.first_writes
    }

    #[must_use]
    pub fn second_writes(&self) -> &[u8] {
        &self.second_writes
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

pub fn run_multi_device_mmio_guest(config: VmConfig) -> Result<MultiDeviceMmioGuestResult, Error> {
    let image = FlatGuestImage::new(
        LONG_MODE_MMIO_GUEST_ENTRY,
        LONG_MODE_MMIO_GUEST_ENTRY,
        &MULTI_DEVICE_GUEST_BYTES,
    )?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModeMmioBootLayout::with_device_mappings(
        memory.region(),
        image.entry(),
        LONG_MODE_MMIO_STACK_POINTER,
        vec![
            LongModeMmioPageMapping::new(LONG_MODE_MMIO_VIRTUAL_PAGE, LONG_MODE_MMIO_DEVICE_GPA),
            LongModeMmioPageMapping::new(MULTI_DEVICE_SECOND_VIRTUAL_PAGE, MULTI_DEVICE_SECOND_GPA),
        ],
    )
    .expect("fixed two-device MMIO page mappings remain valid");
    layout.install_page_tables(&mut memory)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode(layout.boot_layout())?;
    let mut port_io = PortIoBus::with_debug_port();
    let mut mmio = MmioBus::empty();
    mmio.register_byte_device_at(LONG_MODE_MMIO_DEVICE_GPA, MULTI_DEVICE_FIRST_READ_VALUE)
        .expect("fixed first MMIO device registration remains non-overlapping");
    mmio.register_byte_device_at(MULTI_DEVICE_SECOND_GPA, MULTI_DEVICE_SECOND_READ_VALUE)
        .expect("fixed second MMIO device registration remains non-overlapping");

    let execution = run_vcpu_until_stopped_with_mmio(
        &mut vcpu,
        &mut port_io,
        &mut mmio,
        MULTI_DEVICE_EXIT_BUDGET,
    )?;
    validate_mmio_sequence(execution.mmio_exits())?;

    let first_writes = mmio
        .writes_at(LONG_MODE_MMIO_DEVICE_GPA)
        .unwrap_or(&[])
        .to_vec();
    let second_writes = mmio
        .writes_at(MULTI_DEVICE_SECOND_GPA)
        .unwrap_or(&[])
        .to_vec();
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    let report = execution.report();

    if first_writes.as_slice() != [MULTI_DEVICE_FIRST_WRITE_VALUE]
        || second_writes.as_slice() != [MULTI_DEVICE_SECOND_WRITE_VALUE]
        || proof.as_slice() != MULTI_DEVICE_PROOF
        || execution.io_exits().len() != MULTI_DEVICE_PROOF.len()
    {
        return Err(verification_error(format!(
            "expected first writes {:?}, second writes {:?}, and proof {:?}; got first {:?}, second {:?}, proof {:?}",
            [MULTI_DEVICE_FIRST_WRITE_VALUE],
            [MULTI_DEVICE_SECOND_WRITE_VALUE],
            MULTI_DEVICE_PROOF,
            first_writes,
            second_writes,
            proof
        )));
    }

    if report.exit() != VcpuExit::Hlt
        || report.rip() != MULTI_DEVICE_TERMINAL_RIP
        || report.rflags() & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
    {
        return Err(verification_error(format!(
            "expected HLT at RIP {MULTI_DEVICE_TERMINAL_RIP:#x} with architectural RFLAGS bit 1 set, got {report}"
        )));
    }

    Ok(MultiDeviceMmioGuestResult {
        io_exits: execution.io_exits().to_vec(),
        mmio_exits: execution.mmio_exits().to_vec(),
        first_writes,
        second_writes,
        proof,
        report,
    })
}

fn validate_mmio_sequence(exits: &[MmioExit]) -> Result<(), Error> {
    let expected = [
        (
            LONG_MODE_MMIO_DEVICE_GPA,
            MmioDirection::Write,
            Some(MULTI_DEVICE_FIRST_WRITE_VALUE),
        ),
        (LONG_MODE_MMIO_DEVICE_GPA, MmioDirection::Read, None),
        (
            MULTI_DEVICE_SECOND_GPA,
            MmioDirection::Write,
            Some(MULTI_DEVICE_SECOND_WRITE_VALUE),
        ),
        (MULTI_DEVICE_SECOND_GPA, MmioDirection::Read, None),
        (LONG_MODE_MMIO_DEVICE_GPA, MmioDirection::Read, None),
    ];
    if exits.len() != expected.len() {
        return Err(verification_error(format!(
            "expected {} MMIO exits, got {}",
            expected.len(),
            exits.len()
        )));
    }

    for (index, (exit, (address, direction, write_value))) in exits.iter().zip(expected).enumerate()
    {
        let payload_matches = match write_value {
            Some(value) => exit.write_data() == [value],
            None => exit.write_data().is_empty(),
        };
        if exit.address() != address
            || exit.direction() != direction
            || exit.length() != 1
            || !payload_matches
        {
            return Err(verification_error(format!(
                "MMIO exit {index} mismatch: expected address {address:#x}, direction {direction:?}, length 1, write {write_value:?}; got address {:#x}, direction {:?}, length {}, data {:?}",
                exit.address(),
                exit.direction(),
                exit.length(),
                exit.write_data()
            )));
        }
    }
    Ok(())
}

fn verification_error(detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation: "multi-device MMIO execution proof",
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_two_device_guest_contract_is_stable() {
        assert_eq!(MULTI_DEVICE_GUEST_BYTES.len(), 43);
        assert_eq!(MULTI_DEVICE_TERMINAL_RIP, 0x1_002b);
        assert_eq!(MULTI_DEVICE_PROOF, b"ABAM");
        assert_eq!(
            &MULTI_DEVICE_GUEST_BYTES[0..10],
            &[0x48, 0xbb, 0, 0, 0x50, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            &MULTI_DEVICE_GUEST_BYTES[10..20],
            &[0x48, 0xb9, 0, 0x10, 0x50, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            &MULTI_DEVICE_GUEST_BYTES[20..23],
            &[0xc6, 0x03, MULTI_DEVICE_FIRST_WRITE_VALUE]
        );
        assert_eq!(
            &MULTI_DEVICE_GUEST_BYTES[27..30],
            &[0xc6, 0x01, MULTI_DEVICE_SECOND_WRITE_VALUE]
        );
    }
}
