use super::{
    vcpu_operation, Vcpu, VcpuDescriptorTableState, VcpuSegmentState, VcpuSpecialRegisterSnapshot,
    VcpuSpecialRegisterSnapshotComparison,
};
use crate::error::Error;
use crate::kvm::sys;
use crate::long_mode::{
    LongModeBootLayout, LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS,
    LONG_MODE_EFER_REQUIRED_BITS,
};
use std::os::fd::AsRawFd;

const LONG_MODE_CODE_SELECTOR: u16 = 1 << 3;
const LONG_MODE_DATA_SELECTOR: u16 = 2 << 3;
const RFLAGS_RESERVED_BIT: u64 = 1 << 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SegmentEncoding {
    base: u64,
    limit: u32,
    selector: u16,
    segment_type: u8,
    present: u8,
    dpl: u8,
    db: u8,
    s: u8,
    l: u8,
    g: u8,
    avl: u8,
    unusable: u8,
}

impl SegmentEncoding {
    const fn from_state(state: VcpuSegmentState) -> Self {
        Self {
            base: state.base(),
            limit: state.limit(),
            selector: state.selector(),
            segment_type: state.segment_type(),
            present: state.present(),
            dpl: state.dpl(),
            db: state.db(),
            s: state.s(),
            l: state.l(),
            g: state.g(),
            avl: state.avl(),
            unusable: state.unusable(),
        }
    }

    const fn into_kvm_segment(self) -> sys::KvmSegment {
        sys::KvmSegment {
            base: self.base,
            limit: self.limit,
            selector: self.selector,
            type_: self.segment_type,
            present: self.present,
            dpl: self.dpl,
            db: self.db,
            s: self.s,
            l: self.l,
            g: self.g,
            avl: self.avl,
            unusable: self.unusable,
            padding: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DescriptorTableEncoding {
    base: u64,
    limit: u16,
}

impl DescriptorTableEncoding {
    const fn from_state(state: VcpuDescriptorTableState) -> Self {
        Self {
            base: state.base(),
            limit: state.limit(),
        }
    }

    const fn into_kvm_dtable(self) -> sys::KvmDtable {
        sys::KvmDtable {
            base: self.base,
            limit: self.limit,
            padding: [0; 3],
        }
    }
}

fn encode_snapshot(snapshot: &VcpuSpecialRegisterSnapshot) -> sys::KvmSregs {
    sys::KvmSregs {
        cs: SegmentEncoding::from_state(snapshot.cs()).into_kvm_segment(),
        ds: SegmentEncoding::from_state(snapshot.ds()).into_kvm_segment(),
        es: SegmentEncoding::from_state(snapshot.es()).into_kvm_segment(),
        fs: SegmentEncoding::from_state(snapshot.fs()).into_kvm_segment(),
        gs: SegmentEncoding::from_state(snapshot.gs()).into_kvm_segment(),
        ss: SegmentEncoding::from_state(snapshot.ss()).into_kvm_segment(),
        tr: SegmentEncoding::from_state(snapshot.tr()).into_kvm_segment(),
        ldt: SegmentEncoding::from_state(snapshot.ldt()).into_kvm_segment(),
        gdt: DescriptorTableEncoding::from_state(snapshot.gdt()).into_kvm_dtable(),
        idt: DescriptorTableEncoding::from_state(snapshot.idt()).into_kvm_dtable(),
        cr0: snapshot.cr0(),
        cr2: snapshot.cr2(),
        cr3: snapshot.cr3(),
        cr4: snapshot.cr4(),
        cr8: snapshot.cr8(),
        efer: snapshot.efer(),
        apic_base: snapshot.apic_base(),
        interrupt_bitmap: *snapshot.interrupt_bitmap(),
    }
}

fn long_mode_code_segment() -> sys::KvmSegment {
    sys::KvmSegment {
        base: 0,
        limit: u32::MAX,
        selector: LONG_MODE_CODE_SELECTOR,
        type_: 0x0b,
        present: 1,
        dpl: 0,
        db: 0,
        s: 1,
        l: 1,
        g: 1,
        avl: 0,
        unusable: 0,
        padding: 0,
    }
}

fn long_mode_data_segment() -> sys::KvmSegment {
    sys::KvmSegment {
        base: 0,
        limit: u32::MAX,
        selector: LONG_MODE_DATA_SELECTOR,
        type_: 0x03,
        present: 1,
        dpl: 0,
        db: 0,
        s: 1,
        l: 0,
        g: 1,
        avl: 0,
        unusable: 0,
        padding: 0,
    }
}

fn configure_long_mode_sregs(sregs: &mut sys::KvmSregs, layout: &LongModeBootLayout) {
    sregs.cs = long_mode_code_segment();
    let data = long_mode_data_segment();
    sregs.ds = data;
    sregs.es = data;
    sregs.fs = data;
    sregs.gs = data;
    sregs.ss = data;
    sregs.cr0 |= LONG_MODE_CR0_REQUIRED_BITS;
    sregs.cr3 = layout.pml4_address().get();
    sregs.cr4 |= LONG_MODE_CR4_REQUIRED_BITS;
    sregs.efer |= LONG_MODE_EFER_REQUIRED_BITS;
}

fn long_mode_regs(layout: &LongModeBootLayout) -> sys::KvmRegs {
    sys::KvmRegs {
        rsp: layout.stack_pointer(),
        rip: layout.entry().get(),
        rflags: RFLAGS_RESERVED_BIT,
        ..sys::KvmRegs::default()
    }
}

impl Vcpu {
    pub fn initialize_long_mode(&self, layout: &LongModeBootLayout) -> Result<(), Error> {
        let mut sregs = sys::get_sregs(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_SREGS", source))?;
        configure_long_mode_sregs(&mut sregs, layout);
        sys::set_sregs(self.fd.as_raw_fd(), &sregs)
            .map_err(|source| vcpu_operation(self.id, "KVM_SET_SREGS", source))?;

        let regs = long_mode_regs(layout);
        sys::set_regs(self.fd.as_raw_fd(), &regs)
            .map_err(|source| vcpu_operation(self.id, "KVM_SET_REGS", source))?;
        Ok(())
    }

    pub fn restore_special_register_snapshot(
        &self,
        snapshot: &VcpuSpecialRegisterSnapshot,
    ) -> Result<(), Error> {
        let sregs = encode_snapshot(snapshot);
        sys::set_sregs(self.fd.as_raw_fd(), &sregs)
            .map_err(|source| vcpu_operation(self.id, "KVM_SET_SREGS", source))
    }

    pub fn restore_and_verify_special_register_snapshot(
        &self,
        snapshot: &VcpuSpecialRegisterSnapshot,
    ) -> Result<VcpuSpecialRegisterSnapshotComparison, Error> {
        self.restore_special_register_snapshot(snapshot)?;
        let observed = self.capture_special_register_snapshot()?;
        Ok(snapshot.compare(&observed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
    use crate::memory::{GuestMemoryRegion, GuestPhysAddr};

    fn long_mode_layout() -> LongModeBootLayout {
        let memory = GuestMemoryRegion::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)
            .unwrap();
        LongModeBootLayout::new(memory, GuestPhysAddr::new(0x1_0000), 0x1f_f000).unwrap()
    }

    #[test]
    fn segment_encoding_preserves_semantic_fields_and_zeros_padding() {
        let encoded = SegmentEncoding {
            base: 0x1122_3344_5566_7788,
            limit: 0xaabb_ccdd,
            selector: 0x3344,
            segment_type: 0x05,
            present: 0x06,
            dpl: 0x07,
            db: 0x08,
            s: 0x09,
            l: 0x0a,
            g: 0x0b,
            avl: 0x0c,
            unusable: 0x0d,
        }
        .into_kvm_segment();

        assert_eq!(encoded.base, 0x1122_3344_5566_7788);
        assert_eq!(encoded.limit, 0xaabb_ccdd);
        assert_eq!(encoded.selector, 0x3344);
        assert_eq!(encoded.type_, 0x05);
        assert_eq!(encoded.present, 0x06);
        assert_eq!(encoded.dpl, 0x07);
        assert_eq!(encoded.db, 0x08);
        assert_eq!(encoded.s, 0x09);
        assert_eq!(encoded.l, 0x0a);
        assert_eq!(encoded.g, 0x0b);
        assert_eq!(encoded.avl, 0x0c);
        assert_eq!(encoded.unusable, 0x0d);
        assert_eq!(encoded.padding, 0);
    }

    #[test]
    fn descriptor_table_encoding_preserves_semantic_fields_and_zeros_padding() {
        let encoded = DescriptorTableEncoding {
            base: 0x8877_6655_4433_2211,
            limit: 0xbeef,
        }
        .into_kvm_dtable();

        assert_eq!(encoded.base, 0x8877_6655_4433_2211);
        assert_eq!(encoded.limit, 0xbeef);
        assert_eq!(encoded.padding, [0; 3]);
    }

    #[test]
    fn long_mode_sregs_enable_required_control_bits_and_preserve_unrelated_bits() {
        let layout = long_mode_layout();
        let mut sregs = sys::KvmSregs {
            cr0: 1 << 16,
            cr3: 0xdead_beef,
            cr4: 1 << 7,
            efer: 1,
            ..sys::KvmSregs::default()
        };

        configure_long_mode_sregs(&mut sregs, &layout);

        assert_eq!(
            sregs.cr0 & LONG_MODE_CR0_REQUIRED_BITS,
            LONG_MODE_CR0_REQUIRED_BITS
        );
        assert_eq!(sregs.cr0 & (1 << 16), 1 << 16);
        assert_eq!(sregs.cr3, layout.pml4_address().get());
        assert_eq!(
            sregs.cr4 & LONG_MODE_CR4_REQUIRED_BITS,
            LONG_MODE_CR4_REQUIRED_BITS
        );
        assert_eq!(sregs.cr4 & (1 << 7), 1 << 7);
        assert_eq!(
            sregs.efer & LONG_MODE_EFER_REQUIRED_BITS,
            LONG_MODE_EFER_REQUIRED_BITS
        );
        assert_eq!(sregs.efer & 1, 1);
    }

    #[test]
    fn long_mode_segments_are_flat_present_ring_zero_code_and_data() {
        let layout = long_mode_layout();
        let mut sregs = sys::KvmSregs::default();
        configure_long_mode_sregs(&mut sregs, &layout);

        let code = sregs.cs;
        assert_eq!(code.base, 0);
        assert_eq!(code.limit, u32::MAX);
        assert_eq!(code.selector, LONG_MODE_CODE_SELECTOR);
        assert_eq!(code.type_, 0x0b);
        assert_eq!(code.present, 1);
        assert_eq!(code.dpl, 0);
        assert_eq!(code.db, 0);
        assert_eq!(code.s, 1);
        assert_eq!(code.l, 1);
        assert_eq!(code.g, 1);
        assert_eq!(code.unusable, 0);

        let expected_data = long_mode_data_segment();
        assert_eq!(sregs.ds, expected_data);
        assert_eq!(sregs.es, expected_data);
        assert_eq!(sregs.fs, expected_data);
        assert_eq!(sregs.gs, expected_data);
        assert_eq!(sregs.ss, expected_data);
        assert_eq!(expected_data.selector, LONG_MODE_DATA_SELECTOR);
        assert_eq!(expected_data.type_, 0x03);
        assert_eq!(expected_data.l, 0);
    }

    #[test]
    fn long_mode_entry_registers_set_rip_rsp_and_reserved_rflags_bit() {
        let layout = long_mode_layout();
        let regs = long_mode_regs(&layout);

        assert_eq!(regs.rip, layout.entry().get());
        assert_eq!(regs.rsp, layout.stack_pointer());
        assert_eq!(regs.rflags, RFLAGS_RESERVED_BIT);
        assert_eq!(regs.rax, 0);
        assert_eq!(regs.r15, 0);
    }
}
