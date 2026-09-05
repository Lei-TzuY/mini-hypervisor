use super::{vcpu_operation, Vcpu};
use crate::error::{Error, VmExitError};
use crate::interrupt::LongModeInterruptLayout;
use crate::kvm::sys;
use std::io;
use std::os::fd::AsRawFd;

const KVM_EXIT_IRQ_WINDOW_OPEN: u32 = 7;
const APIC_SPIV_OFFSET: usize = 0x0f0;
const APIC_LVT0_OFFSET: usize = 0x350;
const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_DELIVERY_MODE_EXTINT: u32 = 0x700;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LegacyPicExtIntState {
    spiv: u32,
    lint0: u32,
}

impl LegacyPicExtIntState {
    pub(crate) const fn spiv(self) -> u32 {
        self.spiv
    }

    pub(crate) const fn lint0(self) -> u32 {
        self.lint0
    }
}

impl Vcpu {
    pub fn initialize_long_mode_interrupts(
        &self,
        layout: &LongModeInterruptLayout,
    ) -> Result<(), Error> {
        self.initialize_long_mode(layout.boot_layout())?;

        let mut sregs = sys::get_sregs(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_SREGS", source))?;
        configure_interrupt_tables(&mut sregs, layout);
        sys::set_sregs(self.fd.as_raw_fd(), &sregs)
            .map_err(|source| vcpu_operation(self.id, "KVM_SET_SREGS", source))
    }

    pub(crate) fn configure_legacy_pic_extint(&self) -> Result<LegacyPicExtIntState, Error> {
        let mut lapic = sys::get_lapic(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_LAPIC", source))?;
        configure_legacy_pic_extint_state(&mut lapic);
        sys::set_lapic(self.fd.as_raw_fd(), &lapic)
            .map_err(|source| vcpu_operation(self.id, "KVM_SET_LAPIC", source))?;

        let observed = sys::get_lapic(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_LAPIC readback", source))?;
        let state = legacy_pic_extint_state(&observed);
        if !legacy_pic_extint_ready(state) {
            return Err(vcpu_operation(
                self.id,
                "KVM_SET_LAPIC readback verification",
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "expected software-enabled LAPIC with unmasked LINT0 ExtINT; got SPIV {:#x}, LINT0 {:#x}",
                        state.spiv, state.lint0
                    ),
                ),
            ));
        }

        Ok(state)
    }

    pub(crate) fn wait_for_interrupt_window(&mut self) -> Result<(u64, u64), Error> {
        self.set_interrupt_window_request(true);
        let exit_result = self.run_once();
        self.set_interrupt_window_request(false);
        let exit = exit_result?;

        if exit.reason() != KVM_EXIT_IRQ_WINDOW_OPEN {
            return Err(Error::VmExit(VmExitError::UnexpectedSequence {
                stage: "long-mode direct interrupt window",
                expected_reason: KVM_EXIT_IRQ_WINDOW_OPEN,
                actual_reason: exit.reason(),
            }));
        }

        let (ready_for_interrupt_injection, if_flag) = self.interrupt_window_flags();
        if !interrupt_window_ready(ready_for_interrupt_injection, if_flag) {
            return Err(vcpu_operation(
                self.id,
                "KVM_RUN interrupt-window handshake",
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "KVM_EXIT_IRQ_WINDOW_OPEN reported ready_for_interrupt_injection={} and if_flag={}",
                        u8::from(ready_for_interrupt_injection),
                        u8::from(if_flag)
                    ),
                ),
            ));
        }

        let registers = self.registers()?;
        Ok((registers.rip, registers.rflags))
    }

    pub fn inject_interrupt(&self, vector: u8) -> Result<(), Error> {
        let interrupt = sys::KvmInterrupt::new(u32::from(vector));
        sys::inject_interrupt(self.fd.as_raw_fd(), &interrupt)
            .map_err(|source| vcpu_operation(self.id, "KVM_INTERRUPT", source))
    }

    fn set_interrupt_window_request(&mut self, requested: bool) {
        // SAFETY: `KvmRunMapping::map` accepts only a mapping large enough for every required
        // `kvm_run` prefix, which necessarily includes `KvmRunHeader` at offset zero. This method
        // has exclusive access to the vCPU and therefore to the writable shared mapping.
        let header = unsafe { &mut *self.run.ptr.as_ptr().cast::<sys::KvmRunHeader>() };
        set_interrupt_window_request(header, requested);
    }

    fn interrupt_window_flags(&self) -> (bool, bool) {
        // SAFETY: the mapped `kvm_run` region is at least `KvmRunHeader` bytes and KVM places the
        // header at offset zero. The returned booleans are copied immediately; no shared pointer
        // escapes the vCPU boundary.
        let header = unsafe { &*self.run.ptr.as_ptr().cast::<sys::KvmRunHeader>() };
        (
            header.ready_for_interrupt_injection != 0,
            header.if_flag != 0,
        )
    }
}

fn configure_interrupt_tables(sregs: &mut sys::KvmSregs, layout: &LongModeInterruptLayout) {
    sregs.gdt = sys::KvmDtable {
        base: layout.gdt_base().get(),
        limit: layout.gdt_limit(),
        padding: [0; 3],
    };
    sregs.idt = sys::KvmDtable {
        base: layout.idt_base().get(),
        limit: layout.idt_limit(),
        padding: [0; 3],
    };
}

fn configure_legacy_pic_extint_state(state: &mut sys::KvmLapicState) {
    let spiv = read_lapic_register(state, APIC_SPIV_OFFSET);
    write_lapic_register(state, APIC_SPIV_OFFSET, spiv | APIC_SPIV_SOFTWARE_ENABLE);

    let lint0 = read_lapic_register(state, APIC_LVT0_OFFSET);
    let lint0 =
        (lint0 & !(APIC_LVT_MASKED | APIC_LVT_DELIVERY_MODE_MASK)) | APIC_LVT_DELIVERY_MODE_EXTINT;
    write_lapic_register(state, APIC_LVT0_OFFSET, lint0);
}

fn legacy_pic_extint_state(state: &sys::KvmLapicState) -> LegacyPicExtIntState {
    LegacyPicExtIntState {
        spiv: read_lapic_register(state, APIC_SPIV_OFFSET),
        lint0: read_lapic_register(state, APIC_LVT0_OFFSET),
    }
}

const fn legacy_pic_extint_ready(state: LegacyPicExtIntState) -> bool {
    state.spiv & APIC_SPIV_SOFTWARE_ENABLE == APIC_SPIV_SOFTWARE_ENABLE
        && state.lint0 & APIC_LVT_DELIVERY_MODE_MASK == APIC_LVT_DELIVERY_MODE_EXTINT
        && state.lint0 & APIC_LVT_MASKED == 0
}

fn read_lapic_register(state: &sys::KvmLapicState, offset: usize) -> u32 {
    let bytes: [u8; 4] = state.regs[offset..offset + 4]
        .try_into()
        .expect("fixed LAPIC register offset remains inside the 0x400-byte state");
    u32::from_le_bytes(bytes)
}

fn write_lapic_register(state: &mut sys::KvmLapicState, offset: usize, value: u32) {
    state.regs[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn set_interrupt_window_request(header: &mut sys::KvmRunHeader, requested: bool) {
    header.request_interrupt_window = u8::from(requested);
}

const fn interrupt_window_ready(ready_for_interrupt_injection: bool, if_flag: bool) -> bool {
    ready_for_interrupt_injection && if_flag
}

const _: () = {
    assert!(APIC_SPIV_OFFSET + 4 <= sys::KVM_APIC_REG_SIZE);
    assert!(APIC_LVT0_OFFSET + 4 <= sys::KVM_APIC_REG_SIZE);
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interrupt::{
        LONG_MODE_INTERRUPT_GDT_ADDR, LONG_MODE_INTERRUPT_GUEST_ENTRY, LONG_MODE_INTERRUPT_HANDLER,
        LONG_MODE_INTERRUPT_IDT_ADDR, LONG_MODE_INTERRUPT_STACK_POINTER,
        LONG_MODE_INTERRUPT_VECTOR,
    };
    use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
    use crate::memory::{GuestMemoryRegion, GuestPhysAddr};

    fn layout() -> LongModeInterruptLayout {
        LongModeInterruptLayout::new(
            GuestMemoryRegion::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap(),
            LONG_MODE_INTERRUPT_GUEST_ENTRY,
            LONG_MODE_INTERRUPT_STACK_POINTER,
            LONG_MODE_INTERRUPT_VECTOR,
            LONG_MODE_INTERRUPT_HANDLER,
        )
        .unwrap()
    }

    #[test]
    fn interrupt_table_configuration_sets_exact_bases_limits_and_zero_padding() {
        let layout = layout();
        let mut sregs = sys::KvmSregs {
            gdt: sys::KvmDtable {
                base: 0xdead_beef,
                limit: 7,
                padding: [1, 2, 3],
            },
            idt: sys::KvmDtable {
                base: 0xfeed_face,
                limit: 9,
                padding: [4, 5, 6],
            },
            cr0: 0x1234,
            ..sys::KvmSregs::default()
        };

        configure_interrupt_tables(&mut sregs, &layout);

        assert_eq!(sregs.gdt.base, LONG_MODE_INTERRUPT_GDT_ADDR.get());
        assert_eq!(sregs.gdt.limit, 23);
        assert_eq!(sregs.gdt.padding, [0; 3]);
        assert_eq!(sregs.idt.base, LONG_MODE_INTERRUPT_IDT_ADDR.get());
        assert_eq!(sregs.idt.limit, 0x40f);
        assert_eq!(sregs.idt.padding, [0; 3]);
        assert_eq!(sregs.cr0, 0x1234);
    }

    #[test]
    fn legacy_pic_extint_configuration_mutates_only_spiv_and_lint0() {
        let mut state = sys::KvmLapicState {
            regs: [0x5a; sys::KVM_APIC_REG_SIZE],
        };
        write_lapic_register(&mut state, APIC_SPIV_OFFSET, 0x1234_0055);
        write_lapic_register(&mut state, APIC_LVT0_OFFSET, 0xabcd_f2aa);
        let before = state.clone();

        configure_legacy_pic_extint_state(&mut state);

        assert_eq!(
            read_lapic_register(&state, APIC_SPIV_OFFSET),
            0x1234_0055 | APIC_SPIV_SOFTWARE_ENABLE
        );
        assert_eq!(
            read_lapic_register(&state, APIC_LVT0_OFFSET),
            (0xabcd_f2aa & !(APIC_LVT_MASKED | APIC_LVT_DELIVERY_MODE_MASK))
                | APIC_LVT_DELIVERY_MODE_EXTINT
        );
        assert_eq!(
            &state.regs[..APIC_SPIV_OFFSET],
            &before.regs[..APIC_SPIV_OFFSET]
        );
        assert_eq!(
            &state.regs[APIC_SPIV_OFFSET + 4..APIC_LVT0_OFFSET],
            &before.regs[APIC_SPIV_OFFSET + 4..APIC_LVT0_OFFSET]
        );
        assert_eq!(
            &state.regs[APIC_LVT0_OFFSET + 4..],
            &before.regs[APIC_LVT0_OFFSET + 4..]
        );
    }

    #[test]
    fn legacy_pic_extint_readback_requires_enabled_apic_extint_and_unmasked_lint0() {
        let mut state = sys::KvmLapicState::default();
        configure_legacy_pic_extint_state(&mut state);
        let configured = legacy_pic_extint_state(&state);
        assert!(legacy_pic_extint_ready(configured));

        assert!(!legacy_pic_extint_ready(LegacyPicExtIntState {
            spiv: configured.spiv & !APIC_SPIV_SOFTWARE_ENABLE,
            ..configured
        }));
        assert!(!legacy_pic_extint_ready(LegacyPicExtIntState {
            lint0: configured.lint0 | APIC_LVT_MASKED,
            ..configured
        }));
        assert!(!legacy_pic_extint_ready(LegacyPicExtIntState {
            lint0: configured.lint0 & !APIC_LVT_DELIVERY_MODE_MASK,
            ..configured
        }));
    }

    #[test]
    fn interrupt_request_preserves_exact_vector() {
        assert_eq!(
            sys::KvmInterrupt::new(u32::from(LONG_MODE_INTERRUPT_VECTOR)),
            sys::KvmInterrupt {
                irq: u32::from(LONG_MODE_INTERRUPT_VECTOR)
            }
        );
    }

    #[test]
    fn interrupt_window_request_mutates_only_the_kvm_input_flag() {
        let mut header = sys::KvmRunHeader {
            request_interrupt_window: 0,
            immediate_exit: 3,
            padding1: [4; 6],
            exit_reason: 0xdead_beef,
            ready_for_interrupt_injection: 1,
            if_flag: 1,
            flags: 0x1234,
        };
        let preserved = header;

        set_interrupt_window_request(&mut header, true);
        assert_eq!(header.request_interrupt_window, 1);
        assert_eq!(header.immediate_exit, preserved.immediate_exit);
        assert_eq!(header.padding1, preserved.padding1);
        assert_eq!(header.exit_reason, preserved.exit_reason);
        assert_eq!(
            header.ready_for_interrupt_injection,
            preserved.ready_for_interrupt_injection
        );
        assert_eq!(header.if_flag, preserved.if_flag);
        assert_eq!(header.flags, preserved.flags);

        set_interrupt_window_request(&mut header, false);
        assert_eq!(header.request_interrupt_window, 0);
    }

    #[test]
    fn interrupt_window_readiness_requires_kvm_ready_and_guest_if() {
        assert!(interrupt_window_ready(true, true));
        assert!(!interrupt_window_ready(false, true));
        assert!(!interrupt_window_ready(true, false));
        assert!(!interrupt_window_ready(false, false));
        assert_eq!(KVM_EXIT_IRQ_WINDOW_OPEN, 7);
    }
}

include!("mp_state.rs");
