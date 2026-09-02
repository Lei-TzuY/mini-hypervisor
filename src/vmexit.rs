use crate::error::{Error, VmExitError};
use crate::vcpu::{Vcpu, VcpuExit, VcpuId, VcpuRegisters};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmExitReport {
    vcpu_id: VcpuId,
    exit: VcpuExit,
    rip: u64,
    rflags: u64,
}

impl VmExitReport {
    #[must_use]
    pub const fn vcpu_id(self) -> VcpuId {
        self.vcpu_id
    }

    #[must_use]
    pub const fn exit(self) -> VcpuExit {
        self.exit
    }

    #[must_use]
    pub const fn rip(self) -> u64 {
        self.rip
    }

    #[must_use]
    pub const fn rflags(self) -> u64 {
        self.rflags
    }
}

impl fmt::Display for VmExitReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "vCPU {} exit {:?}: rip={:#x}, rflags={:#x}",
            self.vcpu_id.get(),
            self.exit,
            self.rip,
            self.rflags
        )
    }
}

pub fn dispatch_vcpu_exit(vcpu: &Vcpu, exit: VcpuExit) -> Result<VmExitReport, Error> {
    let registers = vcpu.registers()?;
    dispatch_with_registers(vcpu.id(), exit, registers)
}

fn dispatch_with_registers(
    vcpu_id: VcpuId,
    exit: VcpuExit,
    registers: VcpuRegisters,
) -> Result<VmExitReport, Error> {
    match exit {
        VcpuExit::Hlt => Ok(VmExitReport {
            vcpu_id,
            exit,
            rip: registers.rip,
            rflags: registers.rflags,
        }),
        VcpuExit::Unhandled { reason } => Err(Error::VmExit(VmExitError::Unhandled {
            vcpu_id: vcpu_id.get(),
            reason,
            rip: registers.rip,
            rflags: registers.rflags,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTERS: VcpuRegisters = VcpuRegisters {
        rip: 0x1001,
        rflags: 0x2,
    };

    #[test]
    fn hlt_dispatch_produces_structured_report() {
        let report = dispatch_with_registers(VcpuId::BOOT, VcpuExit::Hlt, REGISTERS).unwrap();

        assert_eq!(report.vcpu_id(), VcpuId::BOOT);
        assert_eq!(report.exit(), VcpuExit::Hlt);
        assert_eq!(report.rip(), 0x1001);
        assert_eq!(report.rflags(), 0x2);
    }

    #[test]
    fn unhandled_dispatch_preserves_reason_and_register_context() {
        let result = dispatch_with_registers(
            VcpuId::new(7),
            VcpuExit::Unhandled {
                reason: 0xfeed_beef,
            },
            REGISTERS,
        );

        assert!(matches!(
            result,
            Err(Error::VmExit(VmExitError::Unhandled {
                vcpu_id: 7,
                reason: 0xfeed_beef,
                rip: 0x1001,
                rflags: 0x2,
            }))
        ));
    }
}
