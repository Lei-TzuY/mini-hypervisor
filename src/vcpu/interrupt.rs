use super::{vcpu_operation, Vcpu};
use crate::error::Error;
use crate::interrupt::LongModeInterruptLayout;
use crate::kvm::sys;
use std::os::fd::AsRawFd;

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

    pub fn inject_interrupt(&self, vector: u8) -> Result<(), Error> {
        let interrupt = sys::KvmInterrupt::new(u32::from(vector));
        sys::inject_interrupt(self.fd.as_raw_fd(), &interrupt)
            .map_err(|source| vcpu_operation(self.id, "KVM_INTERRUPT", source))
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
    fn interrupt_request_preserves_exact_vector() {
        assert_eq!(
            sys::KvmInterrupt::new(u32::from(LONG_MODE_INTERRUPT_VECTOR)),
            sys::KvmInterrupt {
                irq: u32::from(LONG_MODE_INTERRUPT_VECTOR)
            }
        );
    }
}
