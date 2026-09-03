use super::{KvmRunMapping, Vcpu, VcpuId};
use crate::error::{Error, VmExitError};
use crate::kvm::sys;

pub(super) const KVM_EXIT_FAIL_ENTRY: u32 = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuFailEntry {
    hardware_entry_failure_reason: u64,
    cpu: u32,
}

impl VcpuFailEntry {
    #[must_use]
    pub const fn hardware_entry_failure_reason(self) -> u64 {
        self.hardware_entry_failure_reason
    }

    #[must_use]
    pub const fn cpu(self) -> u32 {
        self.cpu
    }
}

impl Vcpu {
    pub fn fail_entry(&self) -> Result<VcpuFailEntry, Error> {
        self.run.fail_entry(self.id)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunFailEntry {
    hardware_entry_failure_reason: u64,
    cpu: u32,
    padding: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunFailEntryPrefix {
    header: sys::KvmRunHeader,
    cr8: u64,
    apic_base: u64,
    fail_entry: KvmRunFailEntry,
}

pub(super) const fn required_kvm_run_prefix_size() -> usize {
    std::mem::size_of::<KvmRunFailEntryPrefix>()
}

impl KvmRunMapping {
    fn fail_entry(&self, id: VcpuId) -> Result<VcpuFailEntry, Error> {
        let exit_reason = self.exit_reason();
        if exit_reason != KVM_EXIT_FAIL_ENTRY {
            return Err(Error::VmExit(VmExitError::FailEntryPayloadUnavailable {
                vcpu_id: id.get(),
                exit_reason,
            }));
        }

        debug_assert!(self.len >= required_kvm_run_prefix_size());
        // SAFETY: `KvmRunMapping::map` rejects mappings smaller than every typed prefix used by
        // this crate, KVM places `struct kvm_run` at offset zero, and mmap returns suitably
        // aligned memory.
        let prefix = unsafe { &*self.ptr.as_ptr().cast::<KvmRunFailEntryPrefix>() };
        Ok(decode_fail_entry(prefix.fail_entry))
    }
}

const fn decode_fail_entry(raw: KvmRunFailEntry) -> VcpuFailEntry {
    VcpuFailEntry {
        hardware_entry_failure_reason: raw.hardware_entry_failure_reason,
        cpu: raw.cpu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fail_entry_prefix_matches_kvm_run_union_layout() {
        assert_eq!(std::mem::offset_of!(KvmRunFailEntryPrefix, fail_entry), 32);
        assert_eq!(std::mem::size_of::<KvmRunFailEntry>(), 16);
        assert_eq!(required_kvm_run_prefix_size(), 48);
    }

    #[test]
    fn fail_entry_decoder_copies_hardware_reason_and_cpu_exactly() {
        let decoded = decode_fail_entry(KvmRunFailEntry {
            hardware_entry_failure_reason: 0xfeed_face_cafe_beef,
            cpu: 17,
            padding: 0,
        });

        assert_eq!(
            decoded.hardware_entry_failure_reason(),
            0xfeed_face_cafe_beef
        );
        assert_eq!(decoded.cpu(), 17);
    }
}
