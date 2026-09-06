use super::{vcpu_operation, Vcpu};
use crate::error::Error;
use crate::kvm::sys;
use std::os::fd::AsRawFd;

const APIC_SPIV_OFFSET: usize = 0x0f0;
const APIC_LVTT_OFFSET: usize = 0x320;
const APIC_TMICT_OFFSET: usize = 0x380;
const APIC_TMCCT_OFFSET: usize = 0x390;
const APIC_TDCR_OFFSET: usize = 0x3e0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalApicTimerState {
    spiv: u32,
    lvtt: u32,
    initial_count: u32,
    current_count: u32,
    divide_configuration: u32,
}

impl LocalApicTimerState {
    pub(crate) const fn spiv(self) -> u32 {
        self.spiv
    }

    pub(crate) const fn lvtt(self) -> u32 {
        self.lvtt
    }

    pub(crate) const fn initial_count(self) -> u32 {
        self.initial_count
    }

    pub(crate) const fn current_count(self) -> u32 {
        self.current_count
    }

    pub(crate) const fn divide_configuration(self) -> u32 {
        self.divide_configuration
    }
}

impl Vcpu {
    pub(crate) fn local_apic_timer_state(&self) -> Result<LocalApicTimerState, Error> {
        let lapic = sys::get_lapic(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_LAPIC timer readback", source))?;
        Ok(LocalApicTimerState {
            spiv: read_register(&lapic, APIC_SPIV_OFFSET),
            lvtt: read_register(&lapic, APIC_LVTT_OFFSET),
            initial_count: read_register(&lapic, APIC_TMICT_OFFSET),
            current_count: read_register(&lapic, APIC_TMCCT_OFFSET),
            divide_configuration: read_register(&lapic, APIC_TDCR_OFFSET),
        })
    }
}

fn read_register(state: &sys::KvmLapicState, offset: usize) -> u32 {
    let bytes: [u8; 4] = state.regs[offset..offset + 4]
        .try_into()
        .expect("fixed local-APIC timer register offset remains inside the 0x400-byte state");
    u32::from_le_bytes(bytes)
}

const _: () = {
    assert!(APIC_SPIV_OFFSET + 4 <= sys::KVM_APIC_REG_SIZE);
    assert!(APIC_LVTT_OFFSET + 4 <= sys::KVM_APIC_REG_SIZE);
    assert!(APIC_TMICT_OFFSET + 4 <= sys::KVM_APIC_REG_SIZE);
    assert!(APIC_TMCCT_OFFSET + 4 <= sys::KVM_APIC_REG_SIZE);
    assert!(APIC_TDCR_OFFSET + 4 <= sys::KVM_APIC_REG_SIZE);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_register_offsets_match_xapic_layout() {
        assert_eq!(APIC_LVTT_OFFSET, 0x320);
        assert_eq!(APIC_TMICT_OFFSET, 0x380);
        assert_eq!(APIC_TMCCT_OFFSET, 0x390);
        assert_eq!(APIC_TDCR_OFFSET, 0x3e0);
    }

    #[test]
    fn timer_state_reads_only_fixed_lapic_registers() {
        let mut state = sys::KvmLapicState::default();
        state.regs[APIC_SPIV_OFFSET..APIC_SPIV_OFFSET + 4]
            .copy_from_slice(&0x1ff_u32.to_le_bytes());
        state.regs[APIC_LVTT_OFFSET..APIC_LVTT_OFFSET + 4]
            .copy_from_slice(&0x53_u32.to_le_bytes());
        state.regs[APIC_TMICT_OFFSET..APIC_TMICT_OFFSET + 4]
            .copy_from_slice(&0x10_0000_u32.to_le_bytes());
        state.regs[APIC_TMCCT_OFFSET..APIC_TMCCT_OFFSET + 4]
            .copy_from_slice(&7_u32.to_le_bytes());
        state.regs[APIC_TDCR_OFFSET..APIC_TDCR_OFFSET + 4]
            .copy_from_slice(&0x0b_u32.to_le_bytes());

        assert_eq!(read_register(&state, APIC_SPIV_OFFSET), 0x1ff);
        assert_eq!(read_register(&state, APIC_LVTT_OFFSET), 0x53);
        assert_eq!(read_register(&state, APIC_TMICT_OFFSET), 0x10_0000);
        assert_eq!(read_register(&state, APIC_TMCCT_OFFSET), 7);
        assert_eq!(read_register(&state, APIC_TDCR_OFFSET), 0x0b);
    }
}
