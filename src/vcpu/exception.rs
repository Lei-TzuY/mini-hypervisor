use super::{KvmRunMapping, Vcpu, VcpuId};
use crate::error::{Error, VmExitError};
use crate::kvm::sys;

pub(super) const KVM_EXIT_EXCEPTION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuException {
    exception: u32,
    error_code: u32,
}

impl VcpuException {
    #[must_use]
    pub const fn exception(self) -> u32 {
        self.exception
    }

    #[must_use]
    pub const fn error_code(self) -> u32 {
        self.error_code
    }
}

impl Vcpu {
    pub fn exception_exit(&self) -> Result<VcpuException, Error> {
        self.run.exception(self.id)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunException {
    exception: u32,
    error_code: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunExceptionPrefix {
    header: sys::KvmRunHeader,
    cr8: u64,
    apic_base: u64,
    exception: KvmRunException,
}

pub(super) const fn required_kvm_run_prefix_size() -> usize {
    std::mem::size_of::<KvmRunExceptionPrefix>()
}

impl KvmRunMapping {
    fn exception(&self, id: VcpuId) -> Result<VcpuException, Error> {
        let exit_reason = self.exit_reason();
        if exit_reason != KVM_EXIT_EXCEPTION {
            return Err(Error::VmExit(VmExitError::ExceptionPayloadUnavailable {
                vcpu_id: id.get(),
                exit_reason,
            }));
        }

        debug_assert!(self.len >= required_kvm_run_prefix_size());
        // SAFETY: `KvmRunMapping::map` rejects mappings smaller than every typed prefix used by
        // this crate, KVM places `struct kvm_run` at offset zero, and mmap returns suitably
        // aligned memory.
        let prefix = unsafe { &*self.ptr.as_ptr().cast::<KvmRunExceptionPrefix>() };
        Ok(decode_exception(prefix.exception))
    }
}

const fn decode_exception(raw: KvmRunException) -> VcpuException {
    VcpuException {
        exception: raw.exception,
        error_code: raw.error_code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exception_prefix_matches_kvm_run_union_layout() {
        assert_eq!(std::mem::offset_of!(KvmRunExceptionPrefix, exception), 32);
        assert_eq!(std::mem::size_of::<KvmRunException>(), 8);
        assert_eq!(required_kvm_run_prefix_size(), 40);
    }

    #[test]
    fn exception_decoder_copies_vector_and_error_code_exactly() {
        let decoded = decode_exception(KvmRunException {
            exception: 14,
            error_code: 0x1234,
        });

        assert_eq!(decoded.exception(), 14);
        assert_eq!(decoded.error_code(), 0x1234);
    }
}
