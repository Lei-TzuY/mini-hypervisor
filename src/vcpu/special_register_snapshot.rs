use super::{vcpu_operation, Vcpu};
use crate::error::Error;
use crate::kvm::sys;
use std::os::fd::AsRawFd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuSegmentState {
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

impl VcpuSegmentState {
    const fn from_kvm_segment(segment: sys::KvmSegment) -> Self {
        Self {
            base: segment.base,
            limit: segment.limit,
            selector: segment.selector,
            segment_type: segment.type_,
            present: segment.present,
            dpl: segment.dpl,
            db: segment.db,
            s: segment.s,
            l: segment.l,
            g: segment.g,
            avl: segment.avl,
            unusable: segment.unusable,
        }
    }

    #[must_use]
    pub const fn base(&self) -> u64 {
        self.base
    }

    #[must_use]
    pub const fn limit(&self) -> u32 {
        self.limit
    }

    #[must_use]
    pub const fn selector(&self) -> u16 {
        self.selector
    }

    #[must_use]
    pub const fn segment_type(&self) -> u8 {
        self.segment_type
    }

    #[must_use]
    pub const fn present(&self) -> u8 {
        self.present
    }

    #[must_use]
    pub const fn dpl(&self) -> u8 {
        self.dpl
    }

    #[must_use]
    pub const fn db(&self) -> u8 {
        self.db
    }

    #[must_use]
    pub const fn s(&self) -> u8 {
        self.s
    }

    #[must_use]
    pub const fn l(&self) -> u8 {
        self.l
    }

    #[must_use]
    pub const fn g(&self) -> u8 {
        self.g
    }

    #[must_use]
    pub const fn avl(&self) -> u8 {
        self.avl
    }

    #[must_use]
    pub const fn unusable(&self) -> u8 {
        self.unusable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuDescriptorTableState {
    base: u64,
    limit: u16,
}

impl VcpuDescriptorTableState {
    const fn from_kvm_dtable(table: sys::KvmDtable) -> Self {
        Self {
            base: table.base,
            limit: table.limit,
        }
    }

    #[must_use]
    pub const fn base(&self) -> u64 {
        self.base
    }

    #[must_use]
    pub const fn limit(&self) -> u16 {
        self.limit
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuSpecialRegisterSnapshot {
    cs: VcpuSegmentState,
    ds: VcpuSegmentState,
    es: VcpuSegmentState,
    fs: VcpuSegmentState,
    gs: VcpuSegmentState,
    ss: VcpuSegmentState,
    tr: VcpuSegmentState,
    ldt: VcpuSegmentState,
    gdt: VcpuDescriptorTableState,
    idt: VcpuDescriptorTableState,
    cr0: u64,
    cr2: u64,
    cr3: u64,
    cr4: u64,
    cr8: u64,
    efer: u64,
    apic_base: u64,
    interrupt_bitmap: [u64; 4],
}

impl VcpuSpecialRegisterSnapshot {
    const fn from_kvm_sregs(sregs: sys::KvmSregs) -> Self {
        Self {
            cs: VcpuSegmentState::from_kvm_segment(sregs.cs),
            ds: VcpuSegmentState::from_kvm_segment(sregs.ds),
            es: VcpuSegmentState::from_kvm_segment(sregs.es),
            fs: VcpuSegmentState::from_kvm_segment(sregs.fs),
            gs: VcpuSegmentState::from_kvm_segment(sregs.gs),
            ss: VcpuSegmentState::from_kvm_segment(sregs.ss),
            tr: VcpuSegmentState::from_kvm_segment(sregs.tr),
            ldt: VcpuSegmentState::from_kvm_segment(sregs.ldt),
            gdt: VcpuDescriptorTableState::from_kvm_dtable(sregs.gdt),
            idt: VcpuDescriptorTableState::from_kvm_dtable(sregs.idt),
            cr0: sregs.cr0,
            cr2: sregs.cr2,
            cr3: sregs.cr3,
            cr4: sregs.cr4,
            cr8: sregs.cr8,
            efer: sregs.efer,
            apic_base: sregs.apic_base,
            interrupt_bitmap: sregs.interrupt_bitmap,
        }
    }

    #[must_use]
    pub const fn cs(&self) -> VcpuSegmentState {
        self.cs
    }

    #[must_use]
    pub const fn ds(&self) -> VcpuSegmentState {
        self.ds
    }

    #[must_use]
    pub const fn es(&self) -> VcpuSegmentState {
        self.es
    }

    #[must_use]
    pub const fn fs(&self) -> VcpuSegmentState {
        self.fs
    }

    #[must_use]
    pub const fn gs(&self) -> VcpuSegmentState {
        self.gs
    }

    #[must_use]
    pub const fn ss(&self) -> VcpuSegmentState {
        self.ss
    }

    #[must_use]
    pub const fn tr(&self) -> VcpuSegmentState {
        self.tr
    }

    #[must_use]
    pub const fn ldt(&self) -> VcpuSegmentState {
        self.ldt
    }

    #[must_use]
    pub const fn gdt(&self) -> VcpuDescriptorTableState {
        self.gdt
    }

    #[must_use]
    pub const fn idt(&self) -> VcpuDescriptorTableState {
        self.idt
    }

    #[must_use]
    pub const fn cr0(&self) -> u64 {
        self.cr0
    }

    #[must_use]
    pub const fn cr2(&self) -> u64 {
        self.cr2
    }

    #[must_use]
    pub const fn cr3(&self) -> u64 {
        self.cr3
    }

    #[must_use]
    pub const fn cr4(&self) -> u64 {
        self.cr4
    }

    #[must_use]
    pub const fn cr8(&self) -> u64 {
        self.cr8
    }

    #[must_use]
    pub const fn efer(&self) -> u64 {
        self.efer
    }

    #[must_use]
    pub const fn apic_base(&self) -> u64 {
        self.apic_base
    }

    #[must_use]
    pub const fn interrupt_bitmap(&self) -> &[u64; 4] {
        &self.interrupt_bitmap
    }
}

impl Vcpu {
    pub fn capture_special_register_snapshot(&self) -> Result<VcpuSpecialRegisterSnapshot, Error> {
        let sregs = sys::get_sregs(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_SREGS", source))?;
        Ok(VcpuSpecialRegisterSnapshot::from_kvm_sregs(sregs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(seed: u8, padding: u8) -> sys::KvmSegment {
        sys::KvmSegment {
            base: u64::from(seed) << 32 | u64::from(seed),
            limit: u32::from(seed) * 0x101,
            selector: u16::from(seed) * 0x11,
            type_: seed,
            present: seed.wrapping_add(1),
            dpl: seed.wrapping_add(2),
            db: seed.wrapping_add(3),
            s: seed.wrapping_add(4),
            l: seed.wrapping_add(5),
            g: seed.wrapping_add(6),
            avl: seed.wrapping_add(7),
            unusable: seed.wrapping_add(8),
            padding,
        }
    }

    fn dtable(seed: u16, padding: [u16; 3]) -> sys::KvmDtable {
        sys::KvmDtable {
            base: u64::from(seed) << 32 | u64::from(seed),
            limit: seed,
            padding,
        }
    }

    #[test]
    fn segment_snapshot_copies_semantic_fields_and_ignores_uapi_padding() {
        let mut a = segment(3, 0xaa);
        let mut b = a;
        b.padding = 0x55;

        let a = VcpuSegmentState::from_kvm_segment(a);
        let b = VcpuSegmentState::from_kvm_segment(b);

        assert_eq!(a, b);
        assert_eq!(a.base(), 0x0000_0003_0000_0003);
        assert_eq!(a.limit(), 0x303);
        assert_eq!(a.selector(), 0x33);
        assert_eq!(a.segment_type(), 3);
        assert_eq!(a.present(), 4);
        assert_eq!(a.dpl(), 5);
        assert_eq!(a.db(), 6);
        assert_eq!(a.s(), 7);
        assert_eq!(a.l(), 8);
        assert_eq!(a.g(), 9);
        assert_eq!(a.avl(), 10);
        assert_eq!(a.unusable(), 11);
    }

    #[test]
    fn descriptor_table_snapshot_ignores_uapi_padding() {
        let a = VcpuDescriptorTableState::from_kvm_dtable(dtable(0x1234, [1, 2, 3]));
        let b = VcpuDescriptorTableState::from_kvm_dtable(dtable(0x1234, [4, 5, 6]));

        assert_eq!(a, b);
        assert_eq!(a.base(), 0x0000_1234_0000_1234);
        assert_eq!(a.limit(), 0x1234);
    }

    #[test]
    fn special_register_snapshot_preserves_every_slot_and_scalar() {
        let raw = sys::KvmSregs {
            cs: segment(1, 0xa1),
            ds: segment(2, 0xa2),
            es: segment(3, 0xa3),
            fs: segment(4, 0xa4),
            gs: segment(5, 0xa5),
            ss: segment(6, 0xa6),
            tr: segment(7, 0xa7),
            ldt: segment(8, 0xa8),
            gdt: dtable(0x1111, [1, 2, 3]),
            idt: dtable(0x2222, [4, 5, 6]),
            cr0: 0x10,
            cr2: 0x20,
            cr3: 0x30,
            cr4: 0x40,
            cr8: 0x80,
            efer: 0xe0,
            apic_base: 0xa0,
            interrupt_bitmap: [0x1, 0x2, 0x3, 0x4],
        };

        let snapshot = VcpuSpecialRegisterSnapshot::from_kvm_sregs(raw);

        assert_eq!(snapshot.cs(), VcpuSegmentState::from_kvm_segment(raw.cs));
        assert_eq!(snapshot.ds(), VcpuSegmentState::from_kvm_segment(raw.ds));
        assert_eq!(snapshot.es(), VcpuSegmentState::from_kvm_segment(raw.es));
        assert_eq!(snapshot.fs(), VcpuSegmentState::from_kvm_segment(raw.fs));
        assert_eq!(snapshot.gs(), VcpuSegmentState::from_kvm_segment(raw.gs));
        assert_eq!(snapshot.ss(), VcpuSegmentState::from_kvm_segment(raw.ss));
        assert_eq!(snapshot.tr(), VcpuSegmentState::from_kvm_segment(raw.tr));
        assert_eq!(snapshot.ldt(), VcpuSegmentState::from_kvm_segment(raw.ldt));
        assert_eq!(snapshot.gdt(), VcpuDescriptorTableState::from_kvm_dtable(raw.gdt));
        assert_eq!(snapshot.idt(), VcpuDescriptorTableState::from_kvm_dtable(raw.idt));
        assert_eq!(snapshot.cr0(), 0x10);
        assert_eq!(snapshot.cr2(), 0x20);
        assert_eq!(snapshot.cr3(), 0x30);
        assert_eq!(snapshot.cr4(), 0x40);
        assert_eq!(snapshot.cr8(), 0x80);
        assert_eq!(snapshot.efer(), 0xe0);
        assert_eq!(snapshot.apic_base(), 0xa0);
        assert_eq!(snapshot.interrupt_bitmap(), &[0x1, 0x2, 0x3, 0x4]);
    }
}
