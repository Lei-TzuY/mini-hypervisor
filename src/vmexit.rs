use crate::error::{Error, VmExitError};
use crate::portio::{PortIoBus, PortIoService};
use crate::vcpu::{
    PortIoExit, Vcpu, VcpuExit, VcpuId, VcpuRegisters, VcpuSystemEventType,
};
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
    vcpu: &mut Vcpu,
    exit: VcpuExit,
    port_io: &mut PortIoBus,
) -> Result<VmExitDisposition, Error> {
    match exit {
        VcpuExit::Io => {
            let io = vcpu.port_io_exit()?;
            match port_io.dispatch(&io)? {
                PortIoService::Output => {}
                PortIoService::Input(response) => vcpu.write_port_io_input(&response)?,
            }
            Ok(VmExitDisposition::Continue(io))
        }
        VcpuExit::Hlt | VcpuExit::Shutdown => {
            let registers = vcpu.registers()?;
            Ok(VmExitDisposition::Stopped(stopped_report(
                vcpu.id(),
                exit,
                registers,
            )))
        }
        VcpuExit::SystemEvent => {
            let event = vcpu.system_event()?;
            let registers = vcpu.registers()?;
            Err(unsupported_system_event(
                vcpu.id(),
                event.event_type(),
                event.data(),
                registers,
            ))
        }
        VcpuExit::Unhandled { reason } => {
            let registers = vcpu.registers()?;
            Err(unhandled_exit(vcpu.id(), reason, registers))
        }
    }
}

fn stopped_report(vcpu_id: VcpuId, exit: VcpuExit, registers: VcpuRegisters) -> VmExitReport {
    debug_assert!(matches!(exit, VcpuExit::Hlt | VcpuExit::Shutdown));
    VmExitReport {
        vcpu_id,
        exit,
        rip: registers.rip,
        rflags: registers.rflags,
    }
}

fn unsupported_system_event(
    vcpu_id: VcpuId,
    event_type: VcpuSystemEventType,
    data: &[u64],
    registers: VcpuRegisters,
) -> Error {
    Error::VmExit(VmExitError::UnsupportedSystemEvent {
        vcpu_id: vcpu_id.get(),
        event_type: event_type.raw(),
        data: data.to_vec(),
        rip: registers.rip,
        rflags: registers.rflags,
        exit_reasons: vec![VcpuExit::SystemEvent.reason()],
    })
}

fn unhandled_exit(vcpu_id: VcpuId, reason: u32, registers: VcpuRegisters) -> Error {
    Error::VmExit(VmExitError::Unhandled {
        vcpu_id: vcpu_id.get(),
        reason,
        rip: registers.rip,
        rflags: registers.rflags,
        exit_reasons: vec![reason],
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
    fn terminal_dispatch_reports_hlt_context() {
        let report = stopped_report(VcpuId::BOOT, VcpuExit::Hlt, REGISTERS);

        assert_eq!(report.vcpu_id(), VcpuId::BOOT);
        assert_eq!(report.exit(), VcpuExit::Hlt);
        assert_eq!(report.rip(), 0x1001);
        assert_eq!(report.rflags(), 0x2);
    }

    #[test]
    fn terminal_dispatch_reports_shutdown_context() {
        let report = stopped_report(VcpuId::new(3), VcpuExit::Shutdown, REGISTERS);

        assert_eq!(report.vcpu_id(), VcpuId::new(3));
        assert_eq!(report.exit(), VcpuExit::Shutdown);
        assert_eq!(report.rip(), 0x1001);
        assert_eq!(report.rflags(), 0x2);
    }

    #[test]
    fn system_event_dispatch_preserves_payload_register_context_and_local_trace() {
        let result = unsupported_system_event(
            VcpuId::new(5),
            VcpuSystemEventType::Reset,
            &[0x11, 0x22],
            REGISTERS,
        );

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::UnsupportedSystemEvent {
                vcpu_id: 5,
                event_type: 2,
                data,
                rip: 0x1001,
                rflags: 0x2,
                exit_reasons,
            }) if data == [0x11, 0x22] && exit_reasons == [VcpuExit::SystemEvent.reason()]
        ));
    }

    #[test]
    fn unhandled_dispatch_preserves_reason_register_context_and_local_trace() {
        let result = unhandled_exit(VcpuId::new(7), 0xfeed_beef, REGISTERS);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::Unhandled {
                vcpu_id: 7,
                reason: 0xfeed_beef,
                rip: 0x1001,
                rflags: 0x2,
                exit_reasons,
            }) if exit_reasons == [0xfeed_beef]
        ));
    }
}
