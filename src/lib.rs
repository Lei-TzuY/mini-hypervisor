#![forbid(unsafe_op_in_unsafe_fn)]

pub mod config;
pub mod error;
pub mod kvm;
pub mod vcpu;

use config::VmConfig;
use error::Error;
use kvm::KvmBackend;
use vcpu::VcpuId;

pub fn verify_kvm_lifecycle(config: VmConfig) -> Result<(), Error> {
    let backend = KvmBackend::open()?;
    let vm = backend.create_vm()?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    debug_assert_eq!(vcpu.id(), VcpuId::BOOT);

    Ok(())
}
