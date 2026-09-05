use crate::error::Error;
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::PortIoBus;
use crate::vcpu::{PortIoExit, VcpuId};
use crate::vmexit::VmExitReport;

const TWO_VCPU_RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const TWO_VCPU_RAM_SIZE: u64 = 0x1_0000;
const FIRST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const SECOND_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x2000);
const SHARED_MARKER: GuestPhysAddr = GuestPhysAddr::new(0x3000);
const EXIT_BUDGET_PER_VCPU: u32 = 2;

pub const FIRST_VCPU_ID: VcpuId = VcpuId::BOOT;
pub const SECOND_VCPU_ID: VcpuId = VcpuId::new(1);
pub const FIRST_PROOF: &[u8; 1] = b"0";
pub const SECOND_PROOF: &[u8; 1] = b"1";
pub const SHARED_MARKER_VALUE: u8 = b'A';
pub const FIRST_TERMINAL_RIP: u64 = FIRST_ENTRY.get() + FIRST_GUEST_BYTES.len() as u64;
pub const SECOND_TERMINAL_RIP: u64 = SECOND_ENTRY.get() + 12;

const FIRST_GUEST_BYTES: [u8; 10] = [
    0xb0,
    SHARED_MARKER_VALUE, // mov al, 'A'
    0xa2,
    0x00,
    0x30, // mov [0x3000], al
    0xb0,
    b'0', // mov al, '0'
    0xe6,
    0xe9, // out 0xe9, al
    0xf4, // hlt
];

const SECOND_GUEST_BYTES: [u8; 17] = [
    0xa0,
    0x00,
    0x30, // mov al, [0x3000]
    0x3c,
    SHARED_MARKER_VALUE, // cmp al, 'A'
    0x75,
    0x05, // jne failure
    0xb0,
    b'1', // mov al, '1'
    0xe6,
    0xe9, // out 0xe9, al
    0xf4, // hlt
    0xb0,
    b'F', // failure: mov al, 'F'
    0xe6,
    0xe9, // out 0xe9, al
    0xf4, // hlt
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuGuestResult {
    first_io_exits: Vec<PortIoExit>,
    second_io_exits: Vec<PortIoExit>,
    first_proof: Vec<u8>,
    second_proof: Vec<u8>,
    shared_marker: u8,
    first_report: VmExitReport,
    second_report: VmExitReport,
}

impl TwoVcpuGuestResult {
    #[must_use]
    pub fn first_io_exits(&self) -> &[PortIoExit] {
        &self.first_io_exits
    }

    #[must_use]
    pub fn second_io_exits(&self) -> &[PortIoExit] {
        &self.second_io_exits
    }

    #[must_use]
    pub fn first_proof(&self) -> &[u8] {
        &self.first_proof
    }

    #[must_use]
    pub fn second_proof(&self) -> &[u8] {
        &self.second_proof
    }

    #[must_use]
    pub const fn shared_marker(&self) -> u8 {
        self.shared_marker
    }

    #[must_use]
    pub const fn first_report(&self) -> VmExitReport {
        self.first_report
    }

    #[must_use]
    pub const fn second_report(&self) -> VmExitReport {
        self.second_report
    }
}

pub fn run_two_vcpu_guest() -> Result<TwoVcpuGuestResult, Error> {
    let first_image = FlatGuestImage::new(FIRST_ENTRY, FIRST_ENTRY, &FIRST_GUEST_BYTES)?;
    let second_image = FlatGuestImage::new(SECOND_ENTRY, SECOND_ENTRY, &SECOND_GUEST_BYTES)?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(TWO_VCPU_RAM_BASE, TWO_VCPU_RAM_SIZE)?;
    first_image.load(&mut memory)?;
    second_image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    // Both vCPU objects exist before either executes. This is intentionally a bounded sequential
    // scheduling proof, not a claim of concurrent SMP, INIT/SIPI, or inter-vCPU interrupts.
    let mut first_vcpu = vm.create_vcpu(FIRST_VCPU_ID)?;
    let mut second_vcpu = vm.create_vcpu(SECOND_VCPU_ID)?;
    first_vcpu.initialize_real_mode(first_image.entry())?;
    second_vcpu.initialize_real_mode(second_image.entry())?;

    let mut first_port_io = PortIoBus::with_debug_port();
    let first_execution =
        run_vcpu_until_stopped(&mut first_vcpu, &mut first_port_io, EXIT_BUDGET_PER_VCPU)?;
    let first_proof = first_port_io.debug_output().unwrap_or(&[]).to_vec();

    let mut marker = [0_u8; 1];
    vm.guest_memory()
        .expect("registered two-vCPU guest memory remains owned by the VM")
        .read(SHARED_MARKER, &mut marker)?;

    let mut second_port_io = PortIoBus::with_debug_port();
    let second_execution =
        run_vcpu_until_stopped(&mut second_vcpu, &mut second_port_io, EXIT_BUDGET_PER_VCPU)?;
    let second_proof = second_port_io.debug_output().unwrap_or(&[]).to_vec();

    Ok(TwoVcpuGuestResult {
        first_io_exits: first_execution.io_exits().to_vec(),
        second_io_exits: second_execution.io_exits().to_vec(),
        first_proof,
        second_proof,
        shared_marker: marker[0],
        first_report: first_execution.report(),
        second_report: second_execution.report(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_guest_programs_use_distinct_entries_and_shared_marker() {
        assert_ne!(FIRST_ENTRY, SECOND_ENTRY);
        assert_eq!(&FIRST_GUEST_BYTES[2..5], &[0xa2, 0x00, 0x30]);
        assert_eq!(&SECOND_GUEST_BYTES[0..3], &[0xa0, 0x00, 0x30]);
        assert_eq!(&FIRST_GUEST_BYTES[7..9], &[0xe6, 0xe9]);
        assert_eq!(&SECOND_GUEST_BYTES[9..11], &[0xe6, 0xe9]);
        assert_eq!(FIRST_TERMINAL_RIP, 0x100a);
        assert_eq!(SECOND_TERMINAL_RIP, 0x200c);
    }

    #[test]
    fn second_guest_failure_branch_cannot_alias_success_terminal() {
        assert_eq!(&SECOND_GUEST_BYTES[5..7], &[0x75, 0x05]);
        assert_eq!(&SECOND_GUEST_BYTES[12..14], &[0xb0, b'F']);
        assert_eq!(SECOND_GUEST_BYTES[16], 0xf4);
    }
}
