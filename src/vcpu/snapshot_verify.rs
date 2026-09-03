use super::{
    Vcpu, VcpuRegisterSnapshot, VcpuRegisterSnapshotComparison, VcpuSpecialRegisterSnapshot,
    VcpuSpecialRegisterSnapshotComparison,
};
use crate::error::Error;
use crate::kvm::msr::value_set::{GuestMsrSnapshot, GuestMsrSnapshotComparison};

impl Vcpu {
    pub fn verify_register_snapshot(
        &self,
        snapshot: &VcpuRegisterSnapshot,
    ) -> Result<VcpuRegisterSnapshotComparison, Error> {
        verify_snapshot_with(
            || self.capture_register_snapshot(),
            |observed| snapshot.compare(observed),
        )
    }

    pub fn verify_special_register_snapshot(
        &self,
        snapshot: &VcpuSpecialRegisterSnapshot,
    ) -> Result<VcpuSpecialRegisterSnapshotComparison, Error> {
        verify_snapshot_with(
            || self.capture_special_register_snapshot(),
            |observed| snapshot.compare(observed),
        )
    }

    pub fn verify_msr_snapshot(
        &self,
        snapshot: &GuestMsrSnapshot,
    ) -> Result<GuestMsrSnapshotComparison, Error> {
        verify_snapshot_with(
            || self.capture_msr_snapshot(snapshot.policy()),
            |observed| snapshot.compare(observed),
        )
    }
}

fn verify_snapshot_with<O, C, E, Capture, Compare>(
    mut capture: Capture,
    compare: Compare,
) -> Result<C, E>
where
    Capture: FnMut() -> Result<O, E>,
    Compare: FnOnce(&O) -> C,
{
    let observed = capture()?;
    Ok(compare(&observed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn verification_captures_once_then_compares_observed_state() {
        let captures = Cell::new(0);
        let comparisons = Cell::new(0);

        let result = verify_snapshot_with(
            || {
                captures.set(captures.get() + 1);
                Ok::<_, &'static str>(42_u8)
            },
            |observed| {
                comparisons.set(comparisons.get() + 1);
                *observed + 1
            },
        )
        .unwrap();

        assert_eq!(result, 43);
        assert_eq!(captures.get(), 1);
        assert_eq!(comparisons.get(), 1);
    }

    #[test]
    fn capture_failure_propagates_without_comparison_or_retry() {
        let captures = Cell::new(0);
        let comparisons = Cell::new(0);

        let result = verify_snapshot_with(
            || {
                captures.set(captures.get() + 1);
                Err::<u8, _>("capture failed")
            },
            |_| {
                comparisons.set(comparisons.get() + 1);
                0_u8
            },
        );

        assert_eq!(result, Err("capture failed"));
        assert_eq!(captures.get(), 1);
        assert_eq!(comparisons.get(), 0);
    }
}
