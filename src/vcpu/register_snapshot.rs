use super::{vcpu_operation, Vcpu};
use crate::error::Error;
use crate::kvm::sys;
use std::os::fd::AsRawFd;

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
}
