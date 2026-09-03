use crate::error::{Error, VmExitError};
use crate::portio::PortIoBus;
use crate::vcpu::{PortIoExit, Vcpu, VcpuExit, VcpuId};
use crate::vmexit::{dispatch_vcpu_exit, VmExitDisposition, VmExitReport};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmExecutionResult {
    report: VmExitReport,
    io_exits: Vec<PortIoExit>,
    exit_reasons: Vec<u32>,
    completed_exits: u32,
}

impl VmExecutionResult {
    #[must_use]
    pub const fn report(&self) -> VmExitReport {
        self.report
    }

    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn exit_reasons(&self) -> &[u32] {
        &self.exit_reasons
    }

    #[must_use]
    pub const fn completed_exits(&self) -> u32 {
        self.completed_exits
    }
}

pub fn run_vcpu_until_stopped(
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    exit_budget: u32,
) -> Result<VmExecutionResult, Error> {
    let mut budget = ExitBudget::new(vcpu.id(), exit_budget);
    let mut io_exits = Vec::new();
    let mut exit_reasons = Vec::new();

    loop {
        budget.ensure_run_allowed(&exit_reasons)?;
        let exit = vcpu.run_once()?;
        record_completed_exit(&mut budget, &mut exit_reasons, exit.reason());

        let disposition = dispatch_vcpu_exit(vcpu, exit, port_io)
            .map_err(|error| attach_completed_exit_trace(error, &exit_reasons))?;
        match disposition {
            VmExitDisposition::Continue(io) => io_exits.push(io),
            VmExitDisposition::Stopped(report) => {
                debug_assert_eq!(exit_reasons.len(), budget.completed() as usize);
                debug_assert_eq!(exit_reasons.last().copied(), Some(report.exit().reason()));
                return Ok(VmExecutionResult {
                    report,
                    io_exits,
                    exit_reasons,
                    completed_exits: budget.completed(),
                });
            }
        }
    }
}

fn attach_completed_exit_trace(error: Error, exit_reasons: &[u32]) -> Error {
    match error {
        Error::VmExit(VmExitError::Unhandled {
            vcpu_id,
            reason,
            rip,
            rflags,
            ..
        }) => {
            debug_assert_eq!(exit_reasons.last().copied(), Some(reason));
            Error::VmExit(VmExitError::Unhandled {
                vcpu_id,
                reason,
                rip,
                rflags,
                exit_reasons: exit_reasons.to_vec(),
            })
        }
        Error::VmExit(VmExitError::EntryFailure {
            vcpu_id,
            hardware_entry_failure_reason,
            cpu,
            ..
        }) => {
            debug_assert_eq!(
                exit_reasons.last().copied(),
                Some(VcpuExit::FailEntry.reason())
            );
            Error::VmExit(VmExitError::EntryFailure {
                vcpu_id,
                hardware_entry_failure_reason,
                cpu,
                exit_reasons: exit_reasons.to_vec(),
            })
        }
        Error::VmExit(VmExitError::InternalError {
            vcpu_id, suberror, ..
        }) => {
            debug_assert_eq!(
                exit_reasons.last().copied(),
                Some(VcpuExit::InternalError.reason())
            );
            Error::VmExit(VmExitError::InternalError {
                vcpu_id,
                suberror,
                exit_reasons: exit_reasons.to_vec(),
            })
        }
        Error::VmExit(VmExitError::UnsupportedSystemEvent {
            vcpu_id,
            event_type,
            data,
            rip,
            rflags,
            ..
        }) => {
            debug_assert_eq!(
                exit_reasons.last().copied(),
                Some(VcpuExit::SystemEvent.reason())
            );
            Error::VmExit(VmExitError::UnsupportedSystemEvent {
                vcpu_id,
                event_type,
                data,
                rip,
                rflags,
                exit_reasons: exit_reasons.to_vec(),
            })
        }
        Error::VmExit(VmExitError::InvalidSystemEventDataCount {
            vcpu_id,
            ndata,
            capacity,
            ..
        }) => {
            debug_assert_eq!(
                exit_reasons.last().copied(),
                Some(VcpuExit::SystemEvent.reason())
            );
            Error::VmExit(VmExitError::InvalidSystemEventDataCount {
                vcpu_id,
                ndata,
                capacity,
                exit_reasons: exit_reasons.to_vec(),
            })
        }
        error => error,
    }
}

fn record_completed_exit(budget: &mut ExitBudget, exit_reasons: &mut Vec<u32>, reason: u32) {
    budget.record(reason);
    exit_reasons.push(reason);
    debug_assert_eq!(exit_reasons.len(), budget.completed() as usize);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExitBudget {
    vcpu_id: VcpuId,
    limit: u32,
    completed: u32,
    last_exit_reason: Option<u32>,
}

impl ExitBudget {
    const fn new(vcpu_id: VcpuId, limit: u32) -> Self {
        Self {
            vcpu_id,
            limit,
            completed: 0,
            last_exit_reason: None,
        }
    }

    fn ensure_run_allowed(self, exit_reasons: &[u32]) -> Result<(), Error> {
        debug_assert_eq!(exit_reasons.len(), self.completed as usize);
        debug_assert_eq!(exit_reasons.last().copied(), self.last_exit_reason);

        if self.completed < self.limit {
            return Ok(());
        }

        Err(Error::VmExit(VmExitError::ExitBudgetExhausted {
            vcpu_id: self.vcpu_id.get(),
            budget: self.limit,
            completed: self.completed,
            last_exit_reason: self.last_exit_reason,
            exit_reasons: exit_reasons.to_vec(),
        }))
    }

    fn record(&mut self, reason: u32) {
        debug_assert!(self.completed < self.limit);
        self.completed += 1;
        self.last_exit_reason = Some(reason);
    }

    const fn completed(self) -> u32 {
        self.completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kvm::sys;

    #[test]
    fn zero_budget_rejects_before_any_completed_exit() {
        let budget = ExitBudget::new(VcpuId::new(4), 0);

        assert!(matches!(
            budget.ensure_run_allowed(&[]),
            Err(Error::VmExit(VmExitError::ExitBudgetExhausted {
                vcpu_id: 4,
                budget: 0,
                completed: 0,
                last_exit_reason: None,
                exit_reasons,
            })) if exit_reasons.is_empty()
        ));
    }

    #[test]
    fn exact_budget_allows_each_reserved_run_then_exhausts() {
        let mut budget = ExitBudget::new(VcpuId::BOOT, 2);
        let mut exit_reasons = Vec::new();

        budget.ensure_run_allowed(&exit_reasons).unwrap();
        record_completed_exit(&mut budget, &mut exit_reasons, sys::KVM_EXIT_IO);
        budget.ensure_run_allowed(&exit_reasons).unwrap();
        record_completed_exit(&mut budget, &mut exit_reasons, sys::KVM_EXIT_HLT);

        assert_eq!(budget.completed(), 2);
        assert!(matches!(
            budget.ensure_run_allowed(&exit_reasons),
            Err(Error::VmExit(VmExitError::ExitBudgetExhausted {
                vcpu_id: 0,
                budget: 2,
                completed: 2,
                last_exit_reason: Some(sys::KVM_EXIT_HLT),
                exit_reasons,
            })) if exit_reasons == [sys::KVM_EXIT_IO, sys::KVM_EXIT_HLT]
        ));
    }

    #[test]
    fn exhausted_budget_preserves_last_serviceable_exit_reason() {
        let mut budget = ExitBudget::new(VcpuId::new(3), 1);
        let mut exit_reasons = Vec::new();

        budget.ensure_run_allowed(&exit_reasons).unwrap();
        record_completed_exit(&mut budget, &mut exit_reasons, sys::KVM_EXIT_IO);

        assert!(matches!(
            budget.ensure_run_allowed(&exit_reasons),
            Err(Error::VmExit(VmExitError::ExitBudgetExhausted {
                vcpu_id: 3,
                budget: 1,
                completed: 1,
                last_exit_reason: Some(sys::KVM_EXIT_IO),
                exit_reasons,
            })) if exit_reasons == [sys::KVM_EXIT_IO]
        ));
    }

    #[test]
    fn completed_exit_recording_keeps_budget_and_trace_in_lockstep() {
        let mut budget = ExitBudget::new(VcpuId::BOOT, 2);
        let mut exit_reasons = Vec::new();

        budget.ensure_run_allowed(&exit_reasons).unwrap();
        record_completed_exit(&mut budget, &mut exit_reasons, sys::KVM_EXIT_IO);
        budget.ensure_run_allowed(&exit_reasons).unwrap();
        record_completed_exit(&mut budget, &mut exit_reasons, sys::KVM_EXIT_HLT);

        assert_eq!(exit_reasons, [sys::KVM_EXIT_IO, sys::KVM_EXIT_HLT]);
        assert_eq!(exit_reasons.len(), budget.completed() as usize);
    }

    #[test]
    fn unhandled_error_trace_is_replaced_with_complete_execution_trace() {
        let unknown_reason = 0xfeed_beef;
        let error = Error::VmExit(VmExitError::Unhandled {
            vcpu_id: 7,
            reason: unknown_reason,
            rip: 0x1234,
            rflags: 0x2,
            exit_reasons: vec![unknown_reason],
        });

        let result = attach_completed_exit_trace(error, &[sys::KVM_EXIT_IO, unknown_reason]);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::Unhandled {
                vcpu_id: 7,
                reason: 0xfeed_beef,
                rip: 0x1234,
                rflags: 0x2,
                exit_reasons,
            }) if exit_reasons == [sys::KVM_EXIT_IO, 0xfeed_beef]
        ));
    }

    #[test]
    fn fail_entry_error_trace_is_replaced_with_complete_execution_trace() {
        let fail_entry_reason = VcpuExit::FailEntry.reason();
        let error = Error::VmExit(VmExitError::EntryFailure {
            vcpu_id: 4,
            hardware_entry_failure_reason: 0xdead_beef,
            cpu: 9,
            exit_reasons: vec![fail_entry_reason],
        });

        let result = attach_completed_exit_trace(error, &[sys::KVM_EXIT_IO, fail_entry_reason]);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::EntryFailure {
                vcpu_id: 4,
                hardware_entry_failure_reason: 0xdead_beef,
                cpu: 9,
                exit_reasons,
            }) if exit_reasons == [sys::KVM_EXIT_IO, fail_entry_reason]
        ));
    }

    #[test]
    fn internal_error_trace_is_replaced_with_complete_execution_trace() {
        let internal_error_reason = VcpuExit::InternalError.reason();
        let error = Error::VmExit(VmExitError::InternalError {
            vcpu_id: 5,
            suberror: 4,
            exit_reasons: vec![internal_error_reason],
        });

        let result = attach_completed_exit_trace(error, &[sys::KVM_EXIT_IO, internal_error_reason]);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::InternalError {
                vcpu_id: 5,
                suberror: 4,
                exit_reasons,
            }) if exit_reasons == [sys::KVM_EXIT_IO, internal_error_reason]
        ));
    }

    #[test]
    fn system_event_error_trace_is_replaced_with_complete_execution_trace() {
        let system_event_reason = VcpuExit::SystemEvent.reason();
        let error = Error::VmExit(VmExitError::UnsupportedSystemEvent {
            vcpu_id: 2,
            event_type: 3,
            data: vec![0x11, 0x22],
            rip: 0x2000,
            rflags: 0x2,
            exit_reasons: vec![system_event_reason],
        });

        let result = attach_completed_exit_trace(error, &[sys::KVM_EXIT_IO, system_event_reason]);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::UnsupportedSystemEvent {
                vcpu_id: 2,
                event_type: 3,
                data,
                rip: 0x2000,
                rflags: 0x2,
                exit_reasons,
            }) if data == [0x11, 0x22]
                && exit_reasons == [sys::KVM_EXIT_IO, system_event_reason]
        ));
    }

    #[test]
    fn malformed_system_event_trace_is_replaced_with_complete_execution_trace() {
        let system_event_reason = VcpuExit::SystemEvent.reason();
        let error = Error::VmExit(VmExitError::InvalidSystemEventDataCount {
            vcpu_id: 6,
            ndata: 17,
            capacity: 16,
            exit_reasons: vec![system_event_reason],
        });

        let result = attach_completed_exit_trace(error, &[sys::KVM_EXIT_IO, system_event_reason]);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::InvalidSystemEventDataCount {
                vcpu_id: 6,
                ndata: 17,
                capacity: 16,
                exit_reasons,
            }) if exit_reasons == [sys::KVM_EXIT_IO, system_event_reason]
        ));
    }

    #[test]
    fn trace_annotation_leaves_other_dispatch_errors_unchanged() {
        let error = Error::VmExit(VmExitError::InvalidIoDirection {
            vcpu_id: 3,
            direction: 9,
        });

        let result = attach_completed_exit_trace(error, &[sys::KVM_EXIT_IO]);

        assert!(matches!(
            result,
            Error::VmExit(VmExitError::InvalidIoDirection {
                vcpu_id: 3,
                direction: 9,
            })
        ));
    }
}
