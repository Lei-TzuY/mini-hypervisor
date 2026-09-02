#![forbid(unsafe_op_in_unsafe_fn)]

pub mod config;
pub mod error;
pub mod kvm;
pub mod loader;
pub mod memory;
pub mod vcpu;

use config::VmConfig;
use error::Error;
use kvm::KvmBackend;
use loader::FlatGuestImage;
use memory::{GuestMemory, GuestPhysAddr};
use vcpu::{VcpuExit, VcpuId};

const LIFECYCLE_RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const LIFECYCLE_RAM_SIZE: u64 = 2 * 1024 * 1024;
const HLT_GUEST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const HLT_GUEST_BYTES: [u8; 1] = [0xf4];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HltGuestResult {
    pub exit: VcpuExit,
    pub rip: u64,
    pub rflags: u64,
}

pub fn verify_kvm_lifecycle(config: VmConfig) -> Result<(), Error> {
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    debug_assert_eq!(vcpu.id(), VcpuId::BOOT);

    Ok(())
}

pub fn run_hlt_guest(config: VmConfig) -> Result<HltGuestResult, Error> {
    let image = FlatGuestImage::new(HLT_GUEST_ENTRY, HLT_GUEST_ENTRY, &HLT_GUEST_BYTES)?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_real_mode(image.entry())?;
    let exit = vcpu.run_once()?;
    let registers = vcpu.registers()?;

    Ok(HltGuestResult {
        exit,
        rip: registers.rip,
        rflags: registers.rflags,
    })
}
