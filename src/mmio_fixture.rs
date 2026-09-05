use crate::config::VmConfig;
use crate::error::Error;
use crate::execution::run_vcpu_until_stopped_with_mmio;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::mmio::{MmioBus, BYTE_DEVICE_ADDRESS};
use crate::portio::PortIoBus;
use crate::vcpu::{MmioExit, PortIoExit, VcpuId};
use crate::vmexit::VmExitReport;

pub const MMIO_GUEST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x100);
pub const MMIO_GUEST_RAM_SIZE: u64 = 4096;
pub const MMIO_GUEST_READ_VALUE: u8 = b'R';
pub const MMIO_GUEST_WRITE_VALUE: u8 = b'W';
pub const MMIO_GUEST_PROOF: &[u8; 4] = b"RMIO";
pub const MMIO_GUEST_TERMINAL_RIP: u64 = 0x117;
const MMIO_EXIT_BUDGET: u32 = 7;

const MMIO_GUEST_BYTES: [u8; 23] = [
    0xb0, b'W', // mov al, 'W'
    0xa2, 0x00, 0x20, // mov [0x2000], al -- MMIO write
    0xa0, 0x00, 0x20, // mov al, [0x2000] -- MMIO read
    0xe6, 0xe9, // out 0xe9, al -- 'R'
    0xb0, b'M', 0xe6, 0xe9, // output 'M'
    0xb0, b'I', 0xe6, 0xe9, // output 'I'
    0xb0, b'O', 0xe6, 0xe9, // output 'O'
    0xf4, // hlt
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MmioGuestResult {
    io_exits: Vec<PortIoExit>,
    mmio_exits: Vec<MmioExit>,
    writes: Vec<u8>,
    proof: Vec<u8>,
    report: VmExitReport,
}

impl MmioGuestResult {
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

pub fn run_mmio_guest(config: VmConfig) -> Result<MmioGuestResult, Error> {
    let image = FlatGuestImage::new(MMIO_GUEST_ENTRY, MMIO_GUEST_ENTRY, &MMIO_GUEST_BYTES)?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MMIO_GUEST_RAM_SIZE)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_real_mode(image.entry())?;
    let mut port_io = PortIoBus::with_debug_port();
    let mut mmio = MmioBus::with_byte_device(MMIO_GUEST_READ_VALUE);
    let execution = run_vcpu_until_stopped_with_mmio(
        &mut vcpu,
        &mut port_io,
        &mut mmio,
        MMIO_EXIT_BUDGET,
    )?;

    debug_assert_eq!(execution.completed_exits(), MMIO_EXIT_BUDGET);
    debug_assert_eq!(execution.mmio_exits().len(), 2);
    debug_assert_eq!(execution.io_exits().len(), MMIO_GUEST_PROOF.len());
    let writes = mmio.writes().unwrap_or(&[]).to_vec();
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();

    Ok(MmioGuestResult {
        io_exits: execution.io_exits().to_vec(),
        mmio_exits: execution.mmio_exits().to_vec(),
        writes,
        proof,
        report: execution.report(),
    })
}

#[must_use]
pub const fn mmio_device_address() -> u64 {
    BYTE_DEVICE_ADDRESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmio_guest_machine_code_and_contract_are_stable() {
        assert_eq!(MMIO_GUEST_BYTES.len(), 0x17);
        assert_eq!(MMIO_GUEST_ENTRY.get() + MMIO_GUEST_BYTES.len() as u64, MMIO_GUEST_TERMINAL_RIP);
        assert_eq!(MMIO_GUEST_PROOF, b"RMIO");
        assert_eq!(MMIO_GUEST_WRITE_VALUE, b'W');
        assert_eq!(MMIO_GUEST_READ_VALUE, b'R');
        assert!(BYTE_DEVICE_ADDRESS >= MMIO_GUEST_RAM_SIZE);
        assert_eq!(
            MMIO_GUEST_BYTES,
            [
                0xb0, b'W', 0xa2, 0x00, 0x20, 0xa0, 0x00, 0x20, 0xe6, 0xe9, 0xb0, b'M', 0xe6,
                0xe9, 0xb0, b'I', 0xe6, 0xe9, 0xb0, b'O', 0xe6, 0xe9, 0xf4,
            ]
        );
    }
}
