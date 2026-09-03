use super::{KvmRunMapping, Vcpu, VcpuId};
use crate::error::{Error, VmExitError};
use crate::kvm::sys;

pub(super) const KVM_EXIT_SYSTEM_EVENT: u32 = 24;
const KVM_SYSTEM_EVENT_DATA_CAPACITY: usize = 16;
const KVM_SYSTEM_EVENT_SHUTDOWN: u32 = 1;
const KVM_SYSTEM_EVENT_RESET: u32 = 2;
const KVM_SYSTEM_EVENT_CRASH: u32 = 3;
const KVM_SYSTEM_EVENT_WAKEUP: u32 = 4;
const KVM_SYSTEM_EVENT_SUSPEND: u32 = 5;
const KVM_SYSTEM_EVENT_SEV_TERM: u32 = 6;
const KVM_SYSTEM_EVENT_TDX_FATAL: u32 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuSystemEventType {
    Shutdown,
    Reset,
    Crash,
    Wakeup,
    Suspend,
    SevTerm,
    TdxFatal,
    Unknown(u32),
}

impl VcpuSystemEventType {
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        match raw {
            KVM_SYSTEM_EVENT_SHUTDOWN => Self::Shutdown,
            KVM_SYSTEM_EVENT_RESET => Self::Reset,
            KVM_SYSTEM_EVENT_CRASH => Self::Crash,
            KVM_SYSTEM_EVENT_WAKEUP => Self::Wakeup,
            KVM_SYSTEM_EVENT_SUSPEND => Self::Suspend,
            KVM_SYSTEM_EVENT_SEV_TERM => Self::SevTerm,
            KVM_SYSTEM_EVENT_TDX_FATAL => Self::TdxFatal,
            raw => Self::Unknown(raw),
        }
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        match self {
            Self::Shutdown => KVM_SYSTEM_EVENT_SHUTDOWN,
            Self::Reset => KVM_SYSTEM_EVENT_RESET,
            Self::Crash => KVM_SYSTEM_EVENT_CRASH,
            Self::Wakeup => KVM_SYSTEM_EVENT_WAKEUP,
            Self::Suspend => KVM_SYSTEM_EVENT_SUSPEND,
            Self::SevTerm => KVM_SYSTEM_EVENT_SEV_TERM,
            Self::TdxFatal => KVM_SYSTEM_EVENT_TDX_FATAL,
            Self::Unknown(raw) => raw,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcpuSystemEvent {
    event_type: VcpuSystemEventType,
    data: Vec<u64>,
}

impl VcpuSystemEvent {
    #[must_use]
    pub const fn event_type(&self) -> VcpuSystemEventType {
        self.event_type
    }

    #[must_use]
    pub fn data(&self) -> &[u64] {
        &self.data
    }
}

impl Vcpu {
    pub fn system_event(&self) -> Result<VcpuSystemEvent, Error> {
        self.run.system_event(self.id)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunSystemEvent {
    type_: u32,
    ndata: u32,
    data: [u64; KVM_SYSTEM_EVENT_DATA_CAPACITY],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunSystemEventPrefix {
    header: sys::KvmRunHeader,
    cr8: u64,
    apic_base: u64,
    system_event: KvmRunSystemEvent,
}

pub(super) const fn required_kvm_run_prefix_size() -> usize {
    std::mem::size_of::<KvmRunSystemEventPrefix>()
}

impl KvmRunMapping {
    fn system_event(&self, id: VcpuId) -> Result<VcpuSystemEvent, Error> {
        let exit_reason = self.exit_reason();
        if exit_reason != KVM_EXIT_SYSTEM_EVENT {
            return Err(Error::VmExit(VmExitError::SystemEventPayloadUnavailable {
                vcpu_id: id.get(),
                exit_reason,
            }));
        }

        debug_assert!(self.len >= required_kvm_run_prefix_size());
        // SAFETY: `KvmRunMapping::map` rejects mappings smaller than this prefix, KVM places
        // `struct kvm_run` at offset zero, and mmap returns suitably aligned memory.
        let prefix = unsafe { &*self.ptr.as_ptr().cast::<KvmRunSystemEventPrefix>() };
        decode_system_event(id, prefix.system_event)
    }
}

fn decode_system_event(id: VcpuId, raw: KvmRunSystemEvent) -> Result<VcpuSystemEvent, Error> {
    let data_count = usize::try_from(raw.ndata).expect("u32 system-event count fits usize");
    if data_count > KVM_SYSTEM_EVENT_DATA_CAPACITY {
        return Err(Error::VmExit(VmExitError::InvalidSystemEventDataCount {
            vcpu_id: id.get(),
            ndata: raw.ndata,
            capacity: KVM_SYSTEM_EVENT_DATA_CAPACITY,
            exit_reasons: vec![KVM_EXIT_SYSTEM_EVENT],
        }));
    }

    Ok(VcpuSystemEvent {
        event_type: VcpuSystemEventType::from_raw(raw.type_),
        data: raw.data[..data_count].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_event(type_: u32, ndata: u32, values: &[u64]) -> KvmRunSystemEvent {
        let mut data = [0; KVM_SYSTEM_EVENT_DATA_CAPACITY];
        data[..values.len()].copy_from_slice(values);
        KvmRunSystemEvent { type_, ndata, data }
    }

    #[test]
    fn system_event_prefix_matches_kvm_run_union_offset_and_capacity() {
        assert_eq!(std::mem::offset_of!(KvmRunSystemEventPrefix, system_event), 32);
        assert_eq!(std::mem::size_of::<KvmRunSystemEvent>(), 136);
        assert_eq!(required_kvm_run_prefix_size(), 168);
    }

    #[test]
    fn decodes_only_declared_system_event_data_in_order() {
        let event = decode_system_event(
            VcpuId::new(4),
            raw_event(KVM_SYSTEM_EVENT_RESET, 3, &[10, 20, 30, 40]),
        )
        .unwrap();

        assert_eq!(event.event_type(), VcpuSystemEventType::Reset);
        assert_eq!(event.data(), [10, 20, 30]);
    }

    #[test]
    fn accepts_full_system_event_data_capacity() {
        let values: Vec<u64> = (0..KVM_SYSTEM_EVENT_DATA_CAPACITY as u64).collect();
        let event = decode_system_event(
            VcpuId::BOOT,
            raw_event(
                KVM_SYSTEM_EVENT_CRASH,
                KVM_SYSTEM_EVENT_DATA_CAPACITY as u32,
                &values,
            ),
        )
        .unwrap();

        assert_eq!(event.event_type(), VcpuSystemEventType::Crash);
        assert_eq!(event.data(), values);
    }

    #[test]
    fn rejects_system_event_data_count_above_fixed_kvm_capacity() {
        let error = decode_system_event(
            VcpuId::new(9),
            raw_event(
                KVM_SYSTEM_EVENT_SHUTDOWN,
                KVM_SYSTEM_EVENT_DATA_CAPACITY as u32 + 1,
                &[],
            ),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            Error::VmExit(VmExitError::InvalidSystemEventDataCount {
                vcpu_id: 9,
                ndata: 17,
                capacity: 16,
                exit_reasons,
            }) if exit_reasons == [KVM_EXIT_SYSTEM_EVENT]
        ));
    }

    #[test]
    fn preserves_unknown_system_event_type_and_owns_payload() {
        let event = decode_system_event(
            VcpuId::BOOT,
            raw_event(0xfeed_beef, 2, &[0x1111, 0x2222]),
        )
        .unwrap();

        assert_eq!(event.event_type(), VcpuSystemEventType::Unknown(0xfeed_beef));
        assert_eq!(event.data(), [0x1111, 0x2222]);
    }
}
