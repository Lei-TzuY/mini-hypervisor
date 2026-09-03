use crate::error::Error;
use crate::kvm::msr::value_set::{GuestMsrSnapshot, GuestMsrSnapshotComparison};
use crate::kvm::msr::GuestMsrAccessPolicy;
use crate::vcpu::{
    Vcpu, VcpuRegisterSnapshot, VcpuRegisterSnapshotComparison, VcpuSpecialRegisterSnapshot,
    VcpuSpecialRegisterSnapshotComparison,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcpuStateSnapshot {
    registers: VcpuRegisterSnapshot,
    special_registers: VcpuSpecialRegisterSnapshot,
    msrs: GuestMsrSnapshot,
}

impl VcpuStateSnapshot {
    #[must_use]
    pub const fn registers(&self) -> &VcpuRegisterSnapshot {
        &self.registers
    }

    #[must_use]
    pub const fn special_registers(&self) -> &VcpuSpecialRegisterSnapshot {
        &self.special_registers
    }

    #[must_use]
    pub const fn msrs(&self) -> &GuestMsrSnapshot {
        &self.msrs
    }

    #[must_use]
    pub fn compare(&self, observed: &Self) -> VcpuStateSnapshotComparison {
        let (registers, special_registers, msrs) = compare_components_with(
            || self.registers.compare(&observed.registers),
            || self.special_registers.compare(&observed.special_registers),
            || self.msrs.compare(&observed.msrs),
        );

        VcpuStateSnapshotComparison {
            registers,
            special_registers,
            msrs,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcpuStateSnapshotComparison {
    registers: VcpuRegisterSnapshotComparison,
    special_registers: VcpuSpecialRegisterSnapshotComparison,
    msrs: GuestMsrSnapshotComparison,
}

impl VcpuStateSnapshotComparison {
    #[must_use]
    pub const fn registers(&self) -> &VcpuRegisterSnapshotComparison {
        &self.registers
    }

    #[must_use]
    pub const fn special_registers(&self) -> &VcpuSpecialRegisterSnapshotComparison {
        &self.special_registers
    }

    #[must_use]
    pub const fn msrs(&self) -> &GuestMsrSnapshotComparison {
        &self.msrs
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.registers.is_exact_match()
            && self.special_registers.is_exact_match()
            && self.msrs.is_exact_match()
    }
}

impl Vcpu {
    pub fn capture_state_snapshot(
        &self,
        msr_policy: &GuestMsrAccessPolicy,
    ) -> Result<VcpuStateSnapshot, Error> {
        let (registers, special_registers, msrs) = capture_components_with(
            || self.capture_register_snapshot(),
            || self.capture_special_register_snapshot(),
            || self.capture_msr_snapshot(msr_policy),
        )?;

        Ok(VcpuStateSnapshot {
            registers,
            special_registers,
            msrs,
        })
    }
}

fn capture_components_with<R, S, M, E, FR, FS, FM>(
    mut capture_registers: FR,
    mut capture_special_registers: FS,
    mut capture_msrs: FM,
) -> Result<(R, S, M), E>
where
    FR: FnMut() -> Result<R, E>,
    FS: FnMut() -> Result<S, E>,
    FM: FnMut() -> Result<M, E>,
{
    let registers = capture_registers()?;
    let special_registers = capture_special_registers()?;
    let msrs = capture_msrs()?;
    Ok((registers, special_registers, msrs))
}

fn compare_components_with<R, S, M, FR, FS, FM>(
    mut compare_registers: FR,
    mut compare_special_registers: FS,
    mut compare_msrs: FM,
) -> (R, S, M)
where
    FR: FnMut() -> R,
    FS: FnMut() -> S,
    FM: FnMut() -> M,
{
    let registers = compare_registers();
    let special_registers = compare_special_registers();
    let msrs = compare_msrs();
    (registers, special_registers, msrs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn component_capture_uses_canonical_order_exactly_once() {
        let sequence = RefCell::new(Vec::new());

        let captured = capture_components_with(
            || {
                sequence.borrow_mut().push("registers");
                Ok::<_, &'static str>(1_u8)
            },
            || {
                sequence.borrow_mut().push("special-registers");
                Ok::<_, &'static str>(2_u8)
            },
            || {
                sequence.borrow_mut().push("msrs");
                Ok::<_, &'static str>(3_u8)
            },
        )
        .unwrap();

        assert_eq!(captured, (1, 2, 3));
        assert_eq!(
            &*sequence.borrow(),
            &["registers", "special-registers", "msrs"]
        );
    }

    #[test]
    fn register_capture_failure_skips_remaining_components() {
        let sequence = RefCell::new(Vec::new());

        let error = capture_components_with(
            || {
                sequence.borrow_mut().push("registers");
                Err::<u8, _>("register failure")
            },
            || {
                sequence.borrow_mut().push("special-registers");
                Ok::<_, &'static str>(2_u8)
            },
            || {
                sequence.borrow_mut().push("msrs");
                Ok::<_, &'static str>(3_u8)
            },
        )
        .unwrap_err();

        assert_eq!(error, "register failure");
        assert_eq!(&*sequence.borrow(), &["registers"]);
    }

    #[test]
    fn special_register_capture_failure_skips_msr_capture() {
        let sequence = RefCell::new(Vec::new());

        let error = capture_components_with(
            || {
                sequence.borrow_mut().push("registers");
                Ok::<_, &'static str>(1_u8)
            },
            || {
                sequence.borrow_mut().push("special-registers");
                Err::<u8, _>("special-register failure")
            },
            || {
                sequence.borrow_mut().push("msrs");
                Ok::<_, &'static str>(3_u8)
            },
        )
        .unwrap_err();

        assert_eq!(error, "special-register failure");
        assert_eq!(&*sequence.borrow(), &["registers", "special-registers"]);
    }

    #[test]
    fn component_comparison_uses_canonical_order_exactly_once() {
        let sequence = RefCell::new(Vec::new());

        let compared = compare_components_with(
            || {
                sequence.borrow_mut().push("registers");
                1_u8
            },
            || {
                sequence.borrow_mut().push("special-registers");
                2_u8
            },
            || {
                sequence.borrow_mut().push("msrs");
                3_u8
            },
        );

        assert_eq!(compared, (1, 2, 3));
        assert_eq!(
            &*sequence.borrow(),
            &["registers", "special-registers", "msrs"]
        );
    }
}
