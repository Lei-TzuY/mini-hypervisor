use crate::error::{Error, VmExitError};
use crate::portio::PortIoBus;
use crate::vcpu::{PortIoExit, Vcpu, VcpuExit, VcpuId, VcpuRegisters};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmExitDisposition {
    Continue(PortIoExit),
    Stopped(VmExitReport),
}

pub fn dispatch_vcpu_exit(
    vcpu: &Vcpu,
    exit: VcpuExit,
    port_io: &mut PortIoBus,
) -> Result<VmExitDisposition, Error> {
    match exit {
        VcpuExit::Io => {
            let io = vcpu.port_io_exit()?;
            port_io.dispatch(&io)?;
            Ok(VmExitDisposition::Continue(io))
        }
        VcpuExit::Hlt => {
            let registers = vcpu.registers()?;
            Ok(VmExitDisposition::Stopped(hlt_report(vcpu.id(), registers)))
        }
        VcpuExit::Unhandled { reason } => {
            let registers = vcpu.registers()?;
            Err(unhandled_exit(vcpu.id(), reason, registers))
        }
    }
}

fn hlt_report(vcpu_id: VcpuId, registers: VcpuRegisters) -> VmExitReport {
    VmExitReport {
        vcpu_id,
        exit: VcpuExit::Hlt,
        rip: registers.rip,
        rflags: registers.rflags,
    }
}

fn unhandled_exit(vcpu_id: VcpuId, reason: u32, registers: VcpuRegisters) -> Error {
    Error::VmExit(VmExitError::Unhandled {
        vcpu_id: vcpu_id.get(),
        reason,
        rip: registers.rip,
        rflags: registers.rflags,
    })
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
        let report = hlt_report(VcpuId::BOOT, REGISTERS);

        assert_eq!(report.vcpu_id(), VcpuId::BOOT);
        assert_eq!(report.exit(), VcpuExit::Hlt);
        assert_eq!(report.rip(), 0x1001);
        assert_eq!(report.rflags(), 0x2);
    }

    #[test]
    fn unhandled_dispatch_preserves_reason_and_register_context() {
        let result = unhandled_exit(VcpuId::new(7), 0xfeed_beef, REGISTERS);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::Unhandled {
                vcpu_id: 7,
                reason: 0xfeed_beef,
                rip: 0x1001,
                rflags: 0x2,
            })
        ));
    }
}
