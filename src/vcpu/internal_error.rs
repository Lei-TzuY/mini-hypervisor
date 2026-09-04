use super::{KvmRunMapping, Vcpu, VcpuId};
use crate::error::{Error, VmExitError};
use crate::kvm::sys;

pub(super) const KVM_EXIT_INTERNAL_ERROR: u32 = 17;
const KVM_INTERNAL_ERROR_DATA_CAPACITY: usize = 16;
const KVM_INTERNAL_ERROR_EMULATION: u32 = 1;
const KVM_INTERNAL_ERROR_SIMUL_EX: u32 = 2;
const KVM_INTERNAL_ERROR_DELIVERY_EV: u32 = 3;
const KVM_INTERNAL_ERROR_UNEXPECTED_EXIT_REASON: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuInternalErrorSuberror {
    Emulation,
    SimultaneousExceptions,
    DeliveryEvent,
    UnexpectedExitReason,
    Unknown(u32),
}

impl VcpuInternalErrorSuberror {
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        match raw {
            KVM_INTERNAL_ERROR_EMULATION => Self::Emulation,
            KVM_INTERNAL_ERROR_SIMUL_EX => Self::SimultaneousExceptions,
            KVM_INTERNAL_ERROR_DELIVERY_EV => Self::DeliveryEvent,
            KVM_INTERNAL_ERROR_UNEXPECTED_EXIT_REASON => Self::UnexpectedExitReason,
            raw => Self::Unknown(raw),
        }
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        match self {
            Self::Emulation => KVM_INTERNAL_ERROR_EMULATION,
            Self::SimultaneousExceptions => KVM_INTERNAL_ERROR_SIMUL_EX,
            Self::DeliveryEvent => KVM_INTERNAL_ERROR_DELIVERY_EV,
            Self::UnexpectedExitReason => KVM_INTERNAL_ERROR_UNEXPECTED_EXIT_REASON,
            Self::Unknown(raw) => raw,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuInternalError {
    suberror: u32,
    data_available: bool,
    data_count: usize,
    data: [u64; KVM_INTERNAL_ERROR_DATA_CAPACITY],
}

impl VcpuInternalError {
    #[must_use]
    pub const fn suberror(&self) -> u32 {
        self.suberror
    }

    #[must_use]
    pub const fn suberror_kind(&self) -> VcpuInternalErrorSuberror {
        VcpuInternalErrorSuberror::from_raw(self.suberror)
    }

    #[must_use]
    pub fn data(&self) -> Option<&[u64]> {
        if self.data_available {
            Some(&self.data[..self.data_count])
        } else {
            None
        }
    }
}

impl Vcpu {
    pub fn internal_error(&self) -> Result<VcpuInternalError, Error> {
        self.run
            .internal_error(self.id, self.supports_internal_error_data())
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

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunInternalError {
    suberror: u32,
    ndata: u32,
    data: [u64; KVM_INTERNAL_ERROR_DATA_CAPACITY],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunInternalErrorPrefix {
    header: sys::KvmRunHeader,
    cr8: u64,
    apic_base: u64,
    internal: KvmRunInternalError,
}

pub(super) const fn required_kvm_run_prefix_size() -> usize {
    std::mem::size_of::<KvmRunInternalErrorPrefix>()
}

impl KvmRunMapping {
    fn internal_error(
        &self,
        id: VcpuId,
        supports_optional_data: bool,
    ) -> Result<VcpuInternalError, Error> {
        let exit_reason = self.exit_reason();
        if exit_reason != KVM_EXIT_INTERNAL_ERROR {
            return Err(Error::VmExit(
                VmExitError::InternalErrorPayloadUnavailable {
                    vcpu_id: id.get(),
                    exit_reason,
                },
            ));
        }

        if !supports_optional_data {
            debug_assert!(self.len >= std::mem::size_of::<KvmRunInternalErrorBasePrefix>());
            // SAFETY: `KvmRunMapping::map` rejects mappings smaller than every typed prefix used
            // by this crate, KVM places `struct kvm_run` at offset zero, and mmap returns suitably
            // aligned memory. This base view intentionally ends after the always-available
            // `suberror` field and does not read capability-dependent `ndata` or `data` fields.
            let prefix = unsafe { &*self.ptr.as_ptr().cast::<KvmRunInternalErrorBasePrefix>() };
            return Ok(decode_internal_error_base(prefix.internal));
        }

        debug_assert!(self.len >= required_kvm_run_prefix_size());
        // SAFETY: the mapping is large enough for the full fixed x86 internal-error UAPI prefix,
        // KVM places `struct kvm_run` at offset zero, and mmap returns suitably aligned memory.
        // This full view is formed only after the host reported KVM_CAP_INTERNAL_ERROR_DATA.
        let prefix = unsafe { &*self.ptr.as_ptr().cast::<KvmRunInternalErrorPrefix>() };
        decode_internal_error_with_data(id, prefix.internal)
    }
}

const fn decode_internal_error_base(raw: KvmRunInternalErrorBase) -> VcpuInternalError {
    VcpuInternalError {
        suberror: raw.suberror,
        data_available: false,
        data_count: 0,
        data: [0; KVM_INTERNAL_ERROR_DATA_CAPACITY],
    }
}

fn decode_internal_error_with_data(
    id: VcpuId,
    raw: KvmRunInternalError,
) -> Result<VcpuInternalError, Error> {
    let data_count = usize::try_from(raw.ndata).expect("u32 internal-error count fits usize");
    if data_count > KVM_INTERNAL_ERROR_DATA_CAPACITY {
        return Err(Error::VmExit(VmExitError::InvalidInternalErrorDataCount {
            vcpu_id: id.get(),
            suberror: raw.suberror,
            ndata: raw.ndata,
            capacity: KVM_INTERNAL_ERROR_DATA_CAPACITY,
            exit_reasons: vec![KVM_EXIT_INTERNAL_ERROR],
        }));
    }

    let mut data = [0; KVM_INTERNAL_ERROR_DATA_CAPACITY];
    data[..data_count].copy_from_slice(&raw.data[..data_count]);
    Ok(VcpuInternalError {
        suberror: raw.suberror,
        data_available: true,
        data_count,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_internal_error(suberror: u32, ndata: u32, values: &[u64]) -> KvmRunInternalError {
        let mut data = [0; KVM_INTERNAL_ERROR_DATA_CAPACITY];
        data[..values.len()].copy_from_slice(values);
        KvmRunInternalError {
            suberror,
            ndata,
            data,
        }
    }

    fn instruction_overlay_words(size: u8, bytes: [u8; 15]) -> [u64; 2] {
        let mut overlay = [0_u8; 16];
        overlay[0] = size;
        overlay[1..].copy_from_slice(&bytes);
        [
            u64::from_le_bytes(overlay[..8].try_into().unwrap()),
            u64::from_le_bytes(overlay[8..].try_into().unwrap()),
        ]
    }

    #[test]
    fn internal_error_prefixes_match_kvm_run_union_layout() {
        assert_eq!(
            std::mem::offset_of!(KvmRunInternalErrorBasePrefix, internal),
            32
        );
        assert_eq!(std::mem::size_of::<KvmRunInternalErrorBase>(), 4);
        assert_eq!(std::mem::size_of::<KvmRunInternalError>(), 136);
        assert_eq!(required_kvm_run_prefix_size(), 168);
    }

    #[test]
    fn base_decoder_copies_suberror_without_optional_data() {
        let decoded = decode_internal_error_base(KvmRunInternalErrorBase { suberror: 4 });
        assert_eq!(decoded.suberror(), 4);
        assert_eq!(
            decoded.suberror_kind(),
            VcpuInternalErrorSuberror::UnexpectedExitReason
        );
        assert_eq!(decoded.data(), None);
    }

    #[test]
    fn capability_enabled_decoder_copies_only_declared_data_in_order() {
        let decoded = decode_internal_error_with_data(
            VcpuId::new(4),
            raw_internal_error(2, 3, &[10, 20, 30, 40]),
        )
        .unwrap();

        assert_eq!(decoded.suberror(), 2);
        assert_eq!(
            decoded.suberror_kind(),
            VcpuInternalErrorSuberror::SimultaneousExceptions
        );
        assert_eq!(decoded.data(), Some([10, 20, 30].as_slice()));
    }

    #[test]
    fn capability_enabled_zero_count_is_distinct_from_unavailable_data() {
        let decoded =
            decode_internal_error_with_data(VcpuId::BOOT, raw_internal_error(1, 0, &[])).unwrap();

        assert_eq!(decoded.data(), Some([].as_slice()));
    }

    #[test]
    fn accepts_full_optional_internal_error_data_capacity() {
        let values: Vec<u64> = (0..KVM_INTERNAL_ERROR_DATA_CAPACITY as u64).collect();
        let decoded = decode_internal_error_with_data(
            VcpuId::BOOT,
            raw_internal_error(3, KVM_INTERNAL_ERROR_DATA_CAPACITY as u32, &values),
        )
        .unwrap();

        assert_eq!(decoded.data(), Some(values.as_slice()));
    }

    #[test]
    fn rejects_optional_internal_error_data_count_above_capacity() {
        let error = decode_internal_error_with_data(
            VcpuId::new(9),
            raw_internal_error(4, KVM_INTERNAL_ERROR_DATA_CAPACITY as u32 + 1, &[]),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            Error::VmExit(VmExitError::InvalidInternalErrorDataCount {
                vcpu_id: 9,
                suberror: 4,
                ndata: 17,
                capacity: 16,
                exit_reasons,
            }) if exit_reasons == [KVM_EXIT_INTERNAL_ERROR]
        ));
    }

    #[test]
    fn emulation_metadata_is_absent_without_optional_data_or_for_other_suberrors() {
        let base = decode_internal_error_base(KvmRunInternalErrorBase { suberror: 1 });
        assert_eq!(base.emulation_failure_flags(), None);
        assert_eq!(base.emulation_instruction_size(), None);
        assert_eq!(base.emulation_instruction_bytes(), None);

        let other = decode_internal_error_with_data(
            VcpuId::BOOT,
            raw_internal_error(2, 3, &[1, 2, 3]),
        )
        .unwrap();
        assert_eq!(other.emulation_failure_flags(), None);
        assert_eq!(other.emulation_instruction_size(), None);
        assert_eq!(other.emulation_instruction_bytes(), None);
    }

    #[test]
    fn emulation_failure_flags_preserve_unknown_bits_without_requiring_full_overlay() {
        let flags = (1_u64 << 63) | 1;
        let decoded = decode_internal_error_with_data(
            VcpuId::BOOT,
            raw_internal_error(1, 1, &[flags]),
        )
        .unwrap();

        assert_eq!(decoded.emulation_failure_flags(), Some(flags));
        assert_eq!(decoded.emulation_instruction_size(), None);
        assert_eq!(decoded.emulation_instruction_bytes(), None);
    }

    #[test]
    fn emulation_instruction_bytes_follow_the_fixed_x86_overlay() {
        let mut bytes = [0_u8; 15];
        bytes[..3].copy_from_slice(&[0x90, 0xcc, 0xf4]);
        let words = instruction_overlay_words(3, bytes);
        let decoded = decode_internal_error_with_data(
            VcpuId::BOOT,
            raw_internal_error(1, 3, &[1, words[0], words[1]]),
        )
        .unwrap();

        assert_eq!(decoded.emulation_failure_flags(), Some(1));
        assert_eq!(decoded.emulation_instruction_size(), Some(3));
        assert_eq!(
            decoded.emulation_instruction_bytes(),
            Some(vec![0x90, 0xcc, 0xf4])
        );
    }

    #[test]
    fn emulation_instruction_metadata_requires_flag_and_complete_overlay() {
        let words = instruction_overlay_words(1, [0x90; 15]);
        let without_flag = decode_internal_error_with_data(
            VcpuId::BOOT,
            raw_internal_error(1, 3, &[0, words[0], words[1]]),
        )
        .unwrap();
        assert_eq!(without_flag.emulation_instruction_size(), None);
        assert_eq!(without_flag.emulation_instruction_bytes(), None);

        let incomplete = decode_internal_error_with_data(
            VcpuId::BOOT,
            raw_internal_error(1, 2, &[1, words[0]]),
        )
        .unwrap();
        assert_eq!(incomplete.emulation_instruction_size(), None);
        assert_eq!(incomplete.emulation_instruction_bytes(), None);
    }

    #[test]
    fn oversized_emulation_instruction_size_is_visible_but_never_sliced() {
        let words = instruction_overlay_words(16, [0xaa; 15]);
        let decoded = decode_internal_error_with_data(
            VcpuId::BOOT,
            raw_internal_error(1, 3, &[1, words[0], words[1]]),
        )
        .unwrap();

        assert_eq!(decoded.emulation_instruction_size(), Some(16));
        assert_eq!(decoded.emulation_instruction_bytes(), None);
    }
}
