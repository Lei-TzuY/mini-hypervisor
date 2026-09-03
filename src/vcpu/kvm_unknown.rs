use super::{KvmRunMapping, Vcpu, VcpuId};
use crate::error::{Error, VmExitError};
use crate::kvm::sys;

pub(super) const KVM_EXIT_UNKNOWN: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuKvmUnknownExit {
    hardware_exit_reason: u64,
}

impl VcpuKvmUnknownExit {
    #[must_use]
    pub const fn hardware_exit_reason(self) -> u64 {
        self.hardware_exit_reason
    }
}

impl Vcpu {
    pub fn kvm_unknown_exit(&self) -> Result<VcpuKvmUnknownExit, Error> {
        self.run.kvm_unknown_exit(self.id)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunKvmUnknown {
    hardware_exit_reason: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunKvmUnknownPrefix {
    header: sys::KvmRunHeader,
    cr8: u64,
    apic_base: u64,
    hw: KvmRunKvmUnknown,
}

pub(super) const fn required_kvm_run_prefix_size() -> usize {
    std::mem::size_of::<KvmRunKvmUnknownPrefix>()
}

impl KvmRunMapping {
    fn kvm_unknown_exit(&self, id: VcpuId) -> Result<VcpuKvmUnknownExit, Error> {
        let exit_reason = self.exit_reason();
        if exit_reason != KVM_EXIT_UNKNOWN {
            return Err(Error::VmExit(VmExitError::KvmUnknownPayloadUnavailable {
                vcpu_id: id.get(),
                exit_reason,
            }));
        }

        debug_assert!(self.len >= required_kvm_run_prefix_size());
        // SAFETY: `KvmRunMapping::map` rejects mappings smaller than this prefix, KVM places
        // `struct kvm_run` at offset zero, and mmap returns suitably aligned memory.
        let prefix = unsafe { &*self.ptr.as_ptr().cast::<KvmRunKvmUnknownPrefix>() };
        Ok(VcpuKvmUnknownExit {
            hardware_exit_reason: prefix.hw.hardware_exit_reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kvm_unknown_prefix_matches_kvm_run_union_offset() {
        assert_eq!(std::mem::offset_of!(KvmRunKvmUnknownPrefix, hw), 32);
        assert_eq!(std::mem::size_of::<KvmRunKvmUnknown>(), 8);
        assert_eq!(required_kvm_run_prefix_size(), 40);
    }

    #[test]
    fn typed_kvm_unknown_exit_owns_hardware_reason() {
        let exit = VcpuKvmUnknownExit {
            hardware_exit_reason: 0xfeed_beef_dead_cafe,
        };

        assert_eq!(exit.hardware_exit_reason(), 0xfeed_beef_dead_cafe);
    }
}
