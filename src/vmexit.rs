use crate::error::{Error, VmExitError};
use crate::mmio::{MmioBus, MmioService};
use crate::portio::{PortIoBus, PortIoService};
use crate::vcpu::{
    MmioExit, PortIoExit, Vcpu, VcpuExit, VcpuId, VcpuRegisters, VcpuSystemEventType,
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
pub enum VmExitContinuation {
    PortIo(PortIoExit),
    Mmio(MmioExit),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmExitDisposition {
    Continue(VmExitContinuation),
    Stopped(VmExitReport),
}

pub fn dispatch_vcpu_exit(
    vcpu: &mut Vcpu,
    exit: VcpuExit,
    port_io: &mut PortIoBus,
    mmio: &mut MmioBus,
) -> Result<VmExitDisposition, Error> {
    match exit {
        VcpuExit::Io => {
            let io = vcpu.port_io_exit()?;
            match port_io.dispatch(&io)? {
                PortIoService::Output => {}
                PortIoService::Input(response) => vcpu.write_port_io_input(&response)?,
            }
            Ok(VmExitDisposition::Continue(VmExitContinuation::PortIo(io)))
        }
        VcpuExit::Mmio => {
            let access = vcpu.mmio_exit()?;
            match mmio.dispatch(&access)? {
                MmioService::Write => {}
                MmioService::Read(response) => vcpu.write_mmio_read_response(&response)?,
            }
            Ok(VmExitDisposition::Continue(VmExitContinuation::Mmio(
                access,
            )))
        }
        VcpuExit::Hlt | VcpuExit::Shutdown => {
            let registers = vcpu.registers()?;
            Ok(VmExitDisposition::Stopped(stopped_report(
                vcpu.id(),
                exit,
                registers,
            )))
        }
        VcpuExit::KvmUnknown => {
            let unknown = vcpu.kvm_unknown_exit()?;
            Err(kvm_unknown_exit(vcpu.id(), unknown.hardware_exit_reason()))
        }
        VcpuExit::Exception => {
            let exception = vcpu.exception_exit()?;
            Err(exception_exit(
                vcpu.id(),
                exception.exception(),
                exception.error_code(),
            ))
        }
        VcpuExit::FailEntry => {
            let failure = vcpu.fail_entry()?;
            Err(entry_failure(
                vcpu.id(),
                failure.hardware_entry_failure_reason(),
                failure.cpu(),
            ))
        }
        VcpuExit::InternalError => {
            let internal = vcpu.internal_error()?;
            Err(internal_error(
                vcpu.id(),
                internal.suberror(),
                internal.data(),
            ))
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

fn kvm_unknown_exit(vcpu_id: VcpuId, hardware_exit_reason: u64) -> Error {
    Error::VmExit(VmExitError::KvmUnknownExit {
        vcpu_id: vcpu_id.get(),
        hardware_exit_reason,
        exit_reasons: vec![VcpuExit::KvmUnknown.reason()],
    })
}

fn exception_exit(vcpu_id: VcpuId, exception: u32, error_code: u32) -> Error {
    Error::VmExit(VmExitError::Exception {
        vcpu_id: vcpu_id.get(),
        exception,
        error_code,
        exit_reasons: vec![VcpuExit::Exception.reason()],
    })
}

fn entry_failure(vcpu_id: VcpuId, hardware_entry_failure_reason: u64, cpu: u32) -> Error {
    Error::VmExit(VmExitError::EntryFailure {
        vcpu_id: vcpu_id.get(),
        hardware_entry_failure_reason,
        cpu,
        exit_reasons: vec![VcpuExit::FailEntry.reason()],
    })
}

fn internal_error(vcpu_id: VcpuId, suberror: u32, data: Option<&[u64]>) -> Error {
    Error::VmExit(VmExitError::InternalError {
        vcpu_id: vcpu_id.get(),
        suberror,
        data: data.map(<[u64]>::to_vec),
        exit_reasons: vec![VcpuExit::InternalError.reason()],
    })
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
    fn continuation_variants_preserve_owned_serviceable_exits() {
        let io = PortIoExit::new(crate::vcpu::PortIoDirection::Out, 1, 0xe9, 1, b"K".to_vec());
        assert!(matches!(
            VmExitContinuation::PortIo(io),
            VmExitContinuation::PortIo(exit) if exit.output_data() == b"K"
        ));
    }

    #[test]
    fn kvm_unknown_dispatch_preserves_hardware_reason_and_local_trace_without_register_context() {
        let result = kvm_unknown_exit(VcpuId::new(6), 0xfeed_face_dead_beef);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::KvmUnknownExit {
                vcpu_id: 6,
                hardware_exit_reason: 0xfeed_face_dead_beef,
                exit_reasons,
            }) if exit_reasons == [VcpuExit::KvmUnknown.reason()]
        ));
    }

    #[test]
    fn exception_dispatch_preserves_payload_and_local_trace_without_register_context() {
        let result = exception_exit(VcpuId::new(5), 14, 0x1234);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::Exception {
                vcpu_id: 5,
                exception: 14,
                error_code: 0x1234,
                exit_reasons,
            }) if exit_reasons == [VcpuExit::Exception.reason()]
        ));
    }

    #[test]
    fn fail_entry_dispatch_preserves_payload_and_local_trace_without_register_context() {
        let result = entry_failure(VcpuId::new(6), 0xfeed_face, 11);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::EntryFailure {
                vcpu_id: 6,
                hardware_entry_failure_reason: 0xfeed_face,
                cpu: 11,
                exit_reasons,
            }) if exit_reasons == [VcpuExit::FailEntry.reason()]
        ));
    }

    #[test]
    fn internal_error_dispatch_preserves_absent_optional_data_and_local_trace() {
        let result = internal_error(VcpuId::new(8), 4, None);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::InternalError {
                vcpu_id: 8,
                suberror: 4,
                data: None,
                exit_reasons,
            }) if exit_reasons == [VcpuExit::InternalError.reason()]
        ));
    }

    #[test]
    fn internal_error_dispatch_owns_capability_gated_optional_data() {
        let result = internal_error(VcpuId::new(8), 2, Some(&[0x11, 0x22]));

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::InternalError {
                vcpu_id: 8,
                suberror: 2,
                data: Some(data),
                exit_reasons,
            }) if data == [0x11, 0x22] && exit_reasons == [VcpuExit::InternalError.reason()]
        ));
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
