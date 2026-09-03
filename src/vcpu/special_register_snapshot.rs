use super::{vcpu_operation, Vcpu};
use crate::error::Error;
use crate::kvm::sys;
use std::os::fd::AsRawFd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuSegmentRegister {
    Cs,
    Ds,
    Es,
    Fs,
    Gs,
    Ss,
    Tr,
    Ldt,
}

const SEGMENT_REGISTERS: [VcpuSegmentRegister; 8] = [
    VcpuSegmentRegister::Cs,
    VcpuSegmentRegister::Ds,
    VcpuSegmentRegister::Es,
    VcpuSegmentRegister::Fs,
    VcpuSegmentRegister::Gs,
    VcpuSegmentRegister::Ss,
    VcpuSegmentRegister::Tr,
    VcpuSegmentRegister::Ldt,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuSegmentField {
    Base,
    Limit,
    Selector,
    SegmentType,
    Present,
    Dpl,
    Db,
    S,
    L,
    G,
    Avl,
    Unusable,
}

const SEGMENT_FIELDS: [VcpuSegmentField; 12] = [
    VcpuSegmentField::Base,
    VcpuSegmentField::Limit,
    VcpuSegmentField::Selector,
    VcpuSegmentField::SegmentType,
    VcpuSegmentField::Present,
    VcpuSegmentField::Dpl,
    VcpuSegmentField::Db,
    VcpuSegmentField::S,
    VcpuSegmentField::L,
    VcpuSegmentField::G,
    VcpuSegmentField::Avl,
    VcpuSegmentField::Unusable,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuDescriptorTableRegister {
    Gdt,
    Idt,
}

const DESCRIPTOR_TABLE_REGISTERS: [VcpuDescriptorTableRegister; 2] = [
    VcpuDescriptorTableRegister::Gdt,
    VcpuDescriptorTableRegister::Idt,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuDescriptorTableField {
    Base,
    Limit,
}

const DESCRIPTOR_TABLE_FIELDS: [VcpuDescriptorTableField; 2] = [
    VcpuDescriptorTableField::Base,
    VcpuDescriptorTableField::Limit,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuInterruptBitmapWord {
    Word0,
    Word1,
    Word2,
    Word3,
}

const INTERRUPT_BITMAP_WORDS: [VcpuInterruptBitmapWord; 4] = [
    VcpuInterruptBitmapWord::Word0,
    VcpuInterruptBitmapWord::Word1,
    VcpuInterruptBitmapWord::Word2,
    VcpuInterruptBitmapWord::Word3,
];

impl VcpuInterruptBitmapWord {
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Word0 => 0,
            Self::Word1 => 1,
            Self::Word2 => 2,
            Self::Word3 => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuSpecialRegisterField {
    Segment {
        register: VcpuSegmentRegister,
        field: VcpuSegmentField,
    },
    DescriptorTable {
        register: VcpuDescriptorTableRegister,
        field: VcpuDescriptorTableField,
    },
    Cr0,
    Cr2,
    Cr3,
    Cr4,
    Cr8,
    Efer,
    ApicBase,
    InterruptBitmap(VcpuInterruptBitmapWord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuSpecialRegisterMismatch {
    field: VcpuSpecialRegisterField,
    reference_value: u64,
    observed_value: u64,
}

impl VcpuSpecialRegisterMismatch {
    const fn new(
        field: VcpuSpecialRegisterField,
        reference_value: u64,
        observed_value: u64,
    ) -> Self {
        Self {
            field,
            reference_value,
            observed_value,
        }
    }

    #[must_use]
    pub const fn field(self) -> VcpuSpecialRegisterField {
        self.field
    }

    #[must_use]
    pub const fn reference_value(self) -> u64 {
        self.reference_value
    }

    #[must_use]
    pub const fn observed_value(self) -> u64 {
        self.observed_value
    }
}

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

    const fn value(self, field: VcpuSegmentField) -> u64 {
        match field {
            VcpuSegmentField::Base => self.base,
            VcpuSegmentField::Limit => self.limit as u64,
            VcpuSegmentField::Selector => self.selector as u64,
            VcpuSegmentField::SegmentType => self.segment_type as u64,
            VcpuSegmentField::Present => self.present as u64,
            VcpuSegmentField::Dpl => self.dpl as u64,
            VcpuSegmentField::Db => self.db as u64,
            VcpuSegmentField::S => self.s as u64,
            VcpuSegmentField::L => self.l as u64,
            VcpuSegmentField::G => self.g as u64,
            VcpuSegmentField::Avl => self.avl as u64,
            VcpuSegmentField::Unusable => self.unusable as u64,
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

    const fn value(self, field: VcpuDescriptorTableField) -> u64 {
        match field {
            VcpuDescriptorTableField::Base => self.base,
            VcpuDescriptorTableField::Limit => self.limit as u64,
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

    const fn segment(self, register: VcpuSegmentRegister) -> VcpuSegmentState {
        match register {
            VcpuSegmentRegister::Cs => self.cs,
            VcpuSegmentRegister::Ds => self.ds,
            VcpuSegmentRegister::Es => self.es,
            VcpuSegmentRegister::Fs => self.fs,
            VcpuSegmentRegister::Gs => self.gs,
            VcpuSegmentRegister::Ss => self.ss,
            VcpuSegmentRegister::Tr => self.tr,
            VcpuSegmentRegister::Ldt => self.ldt,
        }
    }

    const fn descriptor_table(
        self,
        register: VcpuDescriptorTableRegister,
    ) -> VcpuDescriptorTableState {
        match register {
            VcpuDescriptorTableRegister::Gdt => self.gdt,
            VcpuDescriptorTableRegister::Idt => self.idt,
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

    #[must_use]
    pub fn compare(&self, observed: &Self) -> VcpuSpecialRegisterSnapshotComparison {
        let mut mismatches = Vec::new();

        for register in SEGMENT_REGISTERS {
            let reference_segment = self.segment(register);
            let observed_segment = observed.segment(register);
            for field in SEGMENT_FIELDS {
                push_mismatch_if_different(
                    &mut mismatches,
                    VcpuSpecialRegisterField::Segment { register, field },
                    reference_segment.value(field),
                    observed_segment.value(field),
                );
            }
        }

        for register in DESCRIPTOR_TABLE_REGISTERS {
            let reference_table = self.descriptor_table(register);
            let observed_table = observed.descriptor_table(register);
            for field in DESCRIPTOR_TABLE_FIELDS {
                push_mismatch_if_different(
                    &mut mismatches,
                    VcpuSpecialRegisterField::DescriptorTable { register, field },
                    reference_table.value(field),
                    observed_table.value(field),
                );
            }
        }

        for (field, reference_value, observed_value) in [
            (VcpuSpecialRegisterField::Cr0, self.cr0, observed.cr0),
            (VcpuSpecialRegisterField::Cr2, self.cr2, observed.cr2),
            (VcpuSpecialRegisterField::Cr3, self.cr3, observed.cr3),
            (VcpuSpecialRegisterField::Cr4, self.cr4, observed.cr4),
            (VcpuSpecialRegisterField::Cr8, self.cr8, observed.cr8),
            (VcpuSpecialRegisterField::Efer, self.efer, observed.efer),
            (
                VcpuSpecialRegisterField::ApicBase,
                self.apic_base,
                observed.apic_base,
            ),
        ] {
            push_mismatch_if_different(&mut mismatches, field, reference_value, observed_value);
        }

        for word in INTERRUPT_BITMAP_WORDS {
            let index = word.index();
            push_mismatch_if_different(
                &mut mismatches,
                VcpuSpecialRegisterField::InterruptBitmap(word),
                self.interrupt_bitmap[index],
                observed.interrupt_bitmap[index],
            );
        }

        VcpuSpecialRegisterSnapshotComparison {
            reference: *self,
            observed: *observed,
            mismatches,
        }
    }
}

fn push_mismatch_if_different(
    mismatches: &mut Vec<VcpuSpecialRegisterMismatch>,
    field: VcpuSpecialRegisterField,
    reference_value: u64,
    observed_value: u64,
) {
    if reference_value != observed_value {
        mismatches.push(VcpuSpecialRegisterMismatch::new(
            field,
            reference_value,
            observed_value,
        ));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcpuSpecialRegisterSnapshotComparison {
    reference: VcpuSpecialRegisterSnapshot,
    observed: VcpuSpecialRegisterSnapshot,
    mismatches: Vec<VcpuSpecialRegisterMismatch>,
}

impl VcpuSpecialRegisterSnapshotComparison {
    #[must_use]
    pub const fn reference(&self) -> &VcpuSpecialRegisterSnapshot {
        &self.reference
    }

    #[must_use]
    pub const fn observed(&self) -> &VcpuSpecialRegisterSnapshot {
        &self.observed
    }

    #[must_use]
    pub fn mismatches(&self) -> &[VcpuSpecialRegisterMismatch] {
        &self.mismatches
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.mismatches.is_empty()
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

    fn special_registers() -> sys::KvmSregs {
        sys::KvmSregs {
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
        }
    }

    #[test]
    fn segment_snapshot_copies_semantic_fields_and_ignores_uapi_padding() {
        let a = segment(3, 0xaa);
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
        let raw = special_registers();
        let snapshot = VcpuSpecialRegisterSnapshot::from_kvm_sregs(raw);

        assert_eq!(snapshot.cs(), VcpuSegmentState::from_kvm_segment(raw.cs));
        assert_eq!(snapshot.ds(), VcpuSegmentState::from_kvm_segment(raw.ds));
        assert_eq!(snapshot.es(), VcpuSegmentState::from_kvm_segment(raw.es));
        assert_eq!(snapshot.fs(), VcpuSegmentState::from_kvm_segment(raw.fs));
        assert_eq!(snapshot.gs(), VcpuSegmentState::from_kvm_segment(raw.gs));
        assert_eq!(snapshot.ss(), VcpuSegmentState::from_kvm_segment(raw.ss));
        assert_eq!(snapshot.tr(), VcpuSegmentState::from_kvm_segment(raw.tr));
        assert_eq!(snapshot.ldt(), VcpuSegmentState::from_kvm_segment(raw.ldt));
        assert_eq!(
            snapshot.gdt(),
            VcpuDescriptorTableState::from_kvm_dtable(raw.gdt)
        );
        assert_eq!(
            snapshot.idt(),
            VcpuDescriptorTableState::from_kvm_dtable(raw.idt)
        );
        assert_eq!(snapshot.cr0(), 0x10);
        assert_eq!(snapshot.cr2(), 0x20);
        assert_eq!(snapshot.cr3(), 0x30);
        assert_eq!(snapshot.cr4(), 0x40);
        assert_eq!(snapshot.cr8(), 0x80);
        assert_eq!(snapshot.efer(), 0xe0);
        assert_eq!(snapshot.apic_base(), 0xa0);
        assert_eq!(snapshot.interrupt_bitmap(), &[0x1, 0x2, 0x3, 0x4]);
    }

    #[test]
    fn identical_special_register_snapshots_compare_as_exact_match() {
        let reference = VcpuSpecialRegisterSnapshot::from_kvm_sregs(special_registers());
        let observed = reference;

        let comparison = reference.compare(&observed);

        assert!(comparison.is_exact_match());
        assert!(comparison.mismatches().is_empty());
        assert_eq!(comparison.reference(), &reference);
        assert_eq!(comparison.observed(), &observed);
    }

    #[test]
    fn nested_segment_difference_reports_typed_field_and_both_values() {
        let reference_raw = special_registers();
        let mut observed_raw = reference_raw;
        observed_raw.fs.dpl = 0xfe;

        let reference = VcpuSpecialRegisterSnapshot::from_kvm_sregs(reference_raw);
        let observed = VcpuSpecialRegisterSnapshot::from_kvm_sregs(observed_raw);
        let comparison = reference.compare(&observed);

        assert_eq!(comparison.mismatches().len(), 1);
        let mismatch = comparison.mismatches()[0];
        assert_eq!(
            mismatch.field(),
            VcpuSpecialRegisterField::Segment {
                register: VcpuSegmentRegister::Fs,
                field: VcpuSegmentField::Dpl,
            }
        );
        assert_eq!(mismatch.reference_value(), reference_raw.fs.dpl as u64);
        assert_eq!(mismatch.observed_value(), 0xfe);
    }

    #[test]
    fn multiple_differences_follow_canonical_semantic_field_order() {
        let reference_raw = special_registers();
        let mut observed_raw = reference_raw;
        observed_raw.cs.base = 0xcafe;
        observed_raw.tr.unusable = 0xee;
        observed_raw.gdt.limit = 0xbeef;
        observed_raw.cr3 = 0xfeed;
        observed_raw.interrupt_bitmap[1] = 0xdead;

        let reference = VcpuSpecialRegisterSnapshot::from_kvm_sregs(reference_raw);
        let observed = VcpuSpecialRegisterSnapshot::from_kvm_sregs(observed_raw);
        let fields: Vec<VcpuSpecialRegisterField> = reference
            .compare(&observed)
            .mismatches()
            .iter()
            .map(|mismatch| mismatch.field())
            .collect();

        assert_eq!(
            fields,
            vec![
                VcpuSpecialRegisterField::Segment {
                    register: VcpuSegmentRegister::Cs,
                    field: VcpuSegmentField::Base,
                },
                VcpuSpecialRegisterField::Segment {
                    register: VcpuSegmentRegister::Tr,
                    field: VcpuSegmentField::Unusable,
                },
                VcpuSpecialRegisterField::DescriptorTable {
                    register: VcpuDescriptorTableRegister::Gdt,
                    field: VcpuDescriptorTableField::Limit,
                },
                VcpuSpecialRegisterField::Cr3,
                VcpuSpecialRegisterField::InterruptBitmap(VcpuInterruptBitmapWord::Word1),
            ]
        );
    }

    #[test]
    fn comparison_owns_complete_source_snapshots_and_is_cloneable() {
        let comparison = {
            let reference = VcpuSpecialRegisterSnapshot::from_kvm_sregs(special_registers());
            let mut observed_raw = special_registers();
            observed_raw.idt.base = 0x4444;
            let observed = VcpuSpecialRegisterSnapshot::from_kvm_sregs(observed_raw);
            reference.compare(&observed)
        };

        let cloned = comparison.clone();

        assert_eq!(cloned, comparison);
        assert_eq!(comparison.reference().cr4(), 0x40);
        assert_eq!(comparison.observed().idt().base(), 0x4444);
        assert_eq!(comparison.mismatches().len(), 1);
    }
}
