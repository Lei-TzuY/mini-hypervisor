use super::{
    vcpu_operation, Vcpu, VcpuDescriptorTableState, VcpuSegmentState, VcpuSpecialRegisterSnapshot,
    VcpuSpecialRegisterSnapshotComparison,
};
use crate::error::Error;
use crate::kvm::sys;
use std::os::fd::AsRawFd;

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

impl Vcpu {
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
}
