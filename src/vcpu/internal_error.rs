use super::{KvmRunMapping, Vcpu, VcpuId};
use crate::error::{Error, VmExitError};
use crate::kvm::sys;

pub(super) const KVM_EXIT_INTERNAL_ERROR: u32 = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuInternalError {
    suberror: u32,
}

impl VcpuInternalError {
    #[must_use]
    pub const fn suberror(self) -> u32 {
        self.suberror
    }
}

impl Vcpu {
    pub fn internal_error(&self) -> Result<VcpuInternalError, Error> {
        self.run.internal_error(self.id)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunInternalErrorBase {
    suberror: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunInternalErrorBasePrefix {
    header: sys::KvmRunHeader,
    cr8: u64,
    apic_base: u64,
    internal: KvmRunInternalErrorBase,
}

pub(super) const fn required_kvm_run_prefix_size() -> usize {
    std::mem::size_of::<KvmRunInternalErrorBasePrefix>()
}

impl KvmRunMapping {
    fn internal_error(&self, id: VcpuId) -> Result<VcpuInternalError, Error> {
        let exit_reason = self.exit_reason();
        if exit_reason != KVM_EXIT_INTERNAL_ERROR {
            return Err(Error::VmExit(
                VmExitError::InternalErrorPayloadUnavailable {
                    vcpu_id: id.get(),
                    exit_reason,
                },
            ));
        }

        debug_assert!(self.len >= required_kvm_run_prefix_size());
        // SAFETY: `KvmRunMapping::map` rejects mappings smaller than every typed prefix used by
        // this crate, KVM places `struct kvm_run` at offset zero, and mmap returns suitably
        // aligned memory. This base view intentionally ends after the always-available `suberror`
        // field and does not read capability-dependent `ndata` or `data` fields.
        let prefix = unsafe { &*self.ptr.as_ptr().cast::<KvmRunInternalErrorBasePrefix>() };
        Ok(decode_internal_error(prefix.internal))
    }
}

const fn decode_internal_error(raw: KvmRunInternalErrorBase) -> VcpuInternalError {
    VcpuInternalError {
        suberror: raw.suberror,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_error_base_prefix_matches_kvm_run_union_layout() {
        assert_eq!(
            std::mem::offset_of!(KvmRunInternalErrorBasePrefix, internal),
            32
        );
        assert_eq!(std::mem::size_of::<KvmRunInternalErrorBase>(), 4);
        assert_eq!(required_kvm_run_prefix_size(), 40);
    }

    #[test]
    fn internal_error_decoder_copies_suberror_only() {
        let decoded = decode_internal_error(KvmRunInternalErrorBase { suberror: 4 });
        assert_eq!(decoded.suberror(), 4);
    }
}
