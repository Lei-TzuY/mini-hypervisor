use super::{vcpu_operation, Vcpu};
use crate::error::Error;
use crate::kvm::sys;
use std::os::fd::AsRawFd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuRegisterField {
    Rax,
    Rbx,
    Rcx,
    Rdx,
    Rsi,
    Rdi,
    Rsp,
    Rbp,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
    Rip,
    Rflags,
}

const REGISTER_FIELDS: [VcpuRegisterField; 18] = [
    VcpuRegisterField::Rax,
    VcpuRegisterField::Rbx,
    VcpuRegisterField::Rcx,
    VcpuRegisterField::Rdx,
    VcpuRegisterField::Rsi,
    VcpuRegisterField::Rdi,
    VcpuRegisterField::Rsp,
    VcpuRegisterField::Rbp,
    VcpuRegisterField::R8,
    VcpuRegisterField::R9,
    VcpuRegisterField::R10,
    VcpuRegisterField::R11,
    VcpuRegisterField::R12,
    VcpuRegisterField::R13,
    VcpuRegisterField::R14,
    VcpuRegisterField::R15,
    VcpuRegisterField::Rip,
    VcpuRegisterField::Rflags,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuRegisterMismatch {
    field: VcpuRegisterField,
    reference_value: u64,
    observed_value: u64,
}

impl VcpuRegisterMismatch {
    const fn new(field: VcpuRegisterField, reference_value: u64, observed_value: u64) -> Self {
        Self {
            field,
            reference_value,
            observed_value,
        }
    }

    #[must_use]
    pub const fn field(self) -> VcpuRegisterField {
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
pub struct VcpuRegisterSnapshot {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rsp: u64,
    rbp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rip: u64,
    rflags: u64,
}

impl VcpuRegisterSnapshot {
    fn from_kvm_regs(regs: sys::KvmRegs) -> Self {
        Self {
            rax: regs.rax,
            rbx: regs.rbx,
            rcx: regs.rcx,
            rdx: regs.rdx,
            rsi: regs.rsi,
            rdi: regs.rdi,
            rsp: regs.rsp,
            rbp: regs.rbp,
            r8: regs.r8,
            r9: regs.r9,
            r10: regs.r10,
            r11: regs.r11,
            r12: regs.r12,
            r13: regs.r13,
            r14: regs.r14,
            r15: regs.r15,
            rip: regs.rip,
            rflags: regs.rflags,
        }
    }

    #[must_use]
    pub const fn rax(&self) -> u64 {
        self.rax
    }

    #[must_use]
    pub const fn rbx(&self) -> u64 {
        self.rbx
    }

    #[must_use]
    pub const fn rcx(&self) -> u64 {
        self.rcx
    }

    #[must_use]
    pub const fn rdx(&self) -> u64 {
        self.rdx
    }

    #[must_use]
    pub const fn rsi(&self) -> u64 {
        self.rsi
    }

    #[must_use]
    pub const fn rdi(&self) -> u64 {
        self.rdi
    }

    #[must_use]
    pub const fn rsp(&self) -> u64 {
        self.rsp
    }

    #[must_use]
    pub const fn rbp(&self) -> u64 {
        self.rbp
    }

    #[must_use]
    pub const fn r8(&self) -> u64 {
        self.r8
    }

    #[must_use]
    pub const fn r9(&self) -> u64 {
        self.r9
    }

    #[must_use]
    pub const fn r10(&self) -> u64 {
        self.r10
    }

    #[must_use]
    pub const fn r11(&self) -> u64 {
        self.r11
    }

    #[must_use]
    pub const fn r12(&self) -> u64 {
        self.r12
    }

    #[must_use]
    pub const fn r13(&self) -> u64 {
        self.r13
    }

    #[must_use]
    pub const fn r14(&self) -> u64 {
        self.r14
    }

    #[must_use]
    pub const fn r15(&self) -> u64 {
        self.r15
    }

    #[must_use]
    pub const fn rip(&self) -> u64 {
        self.rip
    }

    #[must_use]
    pub const fn rflags(&self) -> u64 {
        self.rflags
    }

    #[must_use]
    pub fn compare(&self, observed: &Self) -> VcpuRegisterSnapshotComparison {
        let mut mismatches = Vec::new();
        for field in REGISTER_FIELDS {
            let reference_value = self.value(field);
            let observed_value = observed.value(field);
            if reference_value != observed_value {
                mismatches.push(VcpuRegisterMismatch::new(
                    field,
                    reference_value,
                    observed_value,
                ));
            }
        }

        VcpuRegisterSnapshotComparison {
            reference: *self,
            observed: *observed,
            mismatches,
        }
    }

    const fn value(&self, field: VcpuRegisterField) -> u64 {
        match field {
            VcpuRegisterField::Rax => self.rax,
            VcpuRegisterField::Rbx => self.rbx,
            VcpuRegisterField::Rcx => self.rcx,
            VcpuRegisterField::Rdx => self.rdx,
            VcpuRegisterField::Rsi => self.rsi,
            VcpuRegisterField::Rdi => self.rdi,
            VcpuRegisterField::Rsp => self.rsp,
            VcpuRegisterField::Rbp => self.rbp,
            VcpuRegisterField::R8 => self.r8,
            VcpuRegisterField::R9 => self.r9,
            VcpuRegisterField::R10 => self.r10,
            VcpuRegisterField::R11 => self.r11,
            VcpuRegisterField::R12 => self.r12,
            VcpuRegisterField::R13 => self.r13,
            VcpuRegisterField::R14 => self.r14,
            VcpuRegisterField::R15 => self.r15,
            VcpuRegisterField::Rip => self.rip,
            VcpuRegisterField::Rflags => self.rflags,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcpuRegisterSnapshotComparison {
    reference: VcpuRegisterSnapshot,
    observed: VcpuRegisterSnapshot,
    mismatches: Vec<VcpuRegisterMismatch>,
}

impl VcpuRegisterSnapshotComparison {
    #[must_use]
    pub const fn reference(&self) -> &VcpuRegisterSnapshot {
        &self.reference
    }

    #[must_use]
    pub const fn observed(&self) -> &VcpuRegisterSnapshot {
        &self.observed
    }

    #[must_use]
    pub fn mismatches(&self) -> &[VcpuRegisterMismatch] {
        &self.mismatches
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.mismatches.is_empty()
    }
}

impl Vcpu {
    pub fn capture_register_snapshot(&self) -> Result<VcpuRegisterSnapshot, Error> {
        let regs = sys::get_regs(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_REGS", source))?;
        Ok(VcpuRegisterSnapshot::from_kvm_regs(regs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(values: [u64; 18]) -> VcpuRegisterSnapshot {
        VcpuRegisterSnapshot::from_kvm_regs(sys::KvmRegs {
            rax: values[0],
            rbx: values[1],
            rcx: values[2],
            rdx: values[3],
            rsi: values[4],
            rdi: values[5],
            rsp: values[6],
            rbp: values[7],
            r8: values[8],
            r9: values[9],
            r10: values[10],
            r11: values[11],
            r12: values[12],
            r13: values[13],
            r14: values[14],
            r15: values[15],
            rip: values[16],
            rflags: values[17],
        })
    }

    #[test]
    fn snapshot_copies_every_general_register_field_exactly() {
        let regs = sys::KvmRegs {
            rax: 1,
            rbx: 2,
            rcx: 3,
            rdx: 4,
            rsi: 5,
            rdi: 6,
            rsp: 7,
            rbp: 8,
            r8: 9,
            r9: 10,
            r10: 11,
            r11: 12,
            r12: 13,
            r13: 14,
            r14: 15,
            r15: 16,
            rip: 17,
            rflags: 18,
        };

        let snapshot = VcpuRegisterSnapshot::from_kvm_regs(regs);

        assert_eq!(snapshot.rax(), 1);
        assert_eq!(snapshot.rbx(), 2);
        assert_eq!(snapshot.rcx(), 3);
        assert_eq!(snapshot.rdx(), 4);
        assert_eq!(snapshot.rsi(), 5);
        assert_eq!(snapshot.rdi(), 6);
        assert_eq!(snapshot.rsp(), 7);
        assert_eq!(snapshot.rbp(), 8);
        assert_eq!(snapshot.r8(), 9);
        assert_eq!(snapshot.r9(), 10);
        assert_eq!(snapshot.r10(), 11);
        assert_eq!(snapshot.r11(), 12);
        assert_eq!(snapshot.r12(), 13);
        assert_eq!(snapshot.r13(), 14);
        assert_eq!(snapshot.r14(), 15);
        assert_eq!(snapshot.r15(), 16);
        assert_eq!(snapshot.rip(), 17);
        assert_eq!(snapshot.rflags(), 18);
    }

    #[test]
    fn identical_snapshots_compare_as_exact_match() {
        let reference = snapshot([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]);
        let observed = reference;

        let comparison = reference.compare(&observed);

        assert!(comparison.is_exact_match());
        assert!(comparison.mismatches().is_empty());
        assert_eq!(comparison.reference(), &reference);
        assert_eq!(comparison.observed(), &observed);
    }

    #[test]
    fn one_field_difference_reports_field_and_both_values() {
        let reference = snapshot([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]);
        let observed = snapshot([1, 2, 3, 44, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]);

        let comparison = reference.compare(&observed);

        assert!(!comparison.is_exact_match());
        assert_eq!(comparison.mismatches().len(), 1);
        let mismatch = comparison.mismatches()[0];
        assert_eq!(mismatch.field(), VcpuRegisterField::Rdx);
        assert_eq!(mismatch.reference_value(), 4);
        assert_eq!(mismatch.observed_value(), 44);
    }

    #[test]
    fn multiple_differences_follow_canonical_register_order() {
        let reference = snapshot([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]);
        let observed = snapshot([100, 2, 3, 4, 5, 6, 7, 8, 9, 1000, 11, 12, 13, 14, 15, 16, 1700, 1800]);

        let fields: Vec<VcpuRegisterField> = reference
            .compare(&observed)
            .mismatches()
            .iter()
            .map(|mismatch| mismatch.field())
            .collect();

        assert_eq!(
            fields,
            vec![
                VcpuRegisterField::Rax,
                VcpuRegisterField::R9,
                VcpuRegisterField::Rip,
                VcpuRegisterField::Rflags,
            ]
        );
    }

    #[test]
    fn rip_and_rflags_are_normal_field_mismatches() {
        let reference = snapshot([0; 18]);
        let mut observed_values = [0; 18];
        observed_values[16] = 0x1000;
        observed_values[17] = 0x2;
        let observed = snapshot(observed_values);

        let comparison = reference.compare(&observed);

        assert_eq!(comparison.mismatches().len(), 2);
        assert_eq!(comparison.mismatches()[0].field(), VcpuRegisterField::Rip);
        assert_eq!(comparison.mismatches()[1].field(), VcpuRegisterField::Rflags);
    }

    #[test]
    fn comparison_owns_complete_source_snapshots() {
        let comparison = {
            let reference = snapshot([1; 18]);
            let observed = snapshot([2; 18]);
            reference.compare(&observed)
        };

        assert_eq!(comparison.reference().rax(), 1);
        assert_eq!(comparison.reference().rflags(), 1);
        assert_eq!(comparison.observed().rax(), 2);
        assert_eq!(comparison.observed().rflags(), 2);
        assert_eq!(comparison.mismatches().len(), 18);
    }

    #[test]
    fn comparison_clone_preserves_sources_and_findings() {
        let reference = snapshot([3; 18]);
        let observed = snapshot([4; 18]);
        let comparison = reference.compare(&observed);

        assert_eq!(comparison.clone(), comparison);
    }
}
