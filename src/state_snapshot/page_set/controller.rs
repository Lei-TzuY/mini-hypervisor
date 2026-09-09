use crate::kvm::sys::{KvmLapicState, MasterPicStateSnapshot};
use crate::kvm::Vm;
use crate::vcpu::Vcpu;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedControllerCheckpoint {
    guest: BoundedVcpuPageSetCheckpoint,
    master_pic: MasterPicStateSnapshot,
    lapic: KvmLapicState,
}

impl BoundedControllerCheckpoint {
    pub fn capture(
        vcpu: &Vcpu,
        vm: &Vm,
        msr_policy: &crate::kvm::msr::GuestMsrAccessPolicy,
        memory: &GuestMemory,
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let guest = BoundedVcpuPageSetCheckpoint::capture(
            vcpu,
            msr_policy,
            memory,
            page_addresses,
        )?;
        let master_pic = vm.capture_master_pic_state()?;
        let lapic = vcpu.capture_lapic_checkpoint_state()?;
        Ok(Self {
            guest,
            master_pic,
            lapic,
        })
    }

    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPage] {
        self.guest.pages()
    }

    #[must_use]
    pub(crate) const fn master_pic(&self) -> &MasterPicStateSnapshot {
        &self.master_pic
    }

    #[must_use]
    pub(crate) const fn lapic(&self) -> &KvmLapicState {
        &self.lapic
    }

    pub fn verify(
        &self,
        vcpu: &Vcpu,
        vm: &Vm,
        memory: &GuestMemory,
    ) -> Result<BoundedControllerCheckpointComparison, Error> {
        let guest = self.guest.verify(vcpu, memory)?;
        let master_pic = vm.capture_master_pic_state()? == self.master_pic;
        let lapic = vcpu.capture_lapic_checkpoint_state()? == self.lapic;
        Ok(BoundedControllerCheckpointComparison {
            guest,
            master_pic,
            lapic,
        })
    }

    pub fn restore_and_verify(
        &self,
        vcpu: &Vcpu,
        vm: &Vm,
        memory: &mut GuestMemory,
    ) -> Result<BoundedControllerCheckpointComparison, Error> {
        let guest = self.guest.restore_and_verify(vcpu, memory)?;
        if !guest.is_exact_match() {
            return Err(page_set_error(
                "controller checkpoint guest restore verification",
                "page/VCPU state was not exact after bounded restore; controller restore was not attempted",
            ));
        }

        restore_controller_components_with(
            || vm.restore_master_pic_state(&self.master_pic),
            || vcpu.restore_lapic_checkpoint_state(&self.lapic),
        )?;

        self.verify(vcpu, vm, memory)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedControllerCheckpointComparison {
    guest: BoundedPageSetCheckpointComparison,
    master_pic: bool,
    lapic: bool,
}

impl BoundedControllerCheckpointComparison {
    #[must_use]
    pub const fn guest(&self) -> &BoundedPageSetCheckpointComparison {
        &self.guest
    }

    #[must_use]
    pub fn page_exact(&self, address: GuestPhysAddr) -> Option<bool> {
        self.guest.page_exact(address)
    }

    #[must_use]
    pub fn vcpu_exact(&self) -> bool {
        self.guest.vcpu().is_exact_match()
    }

    #[must_use]
    pub const fn master_pic_exact(&self) -> bool {
        self.master_pic
    }

    #[must_use]
    pub const fn lapic_exact(&self) -> bool {
        self.lapic
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.guest.is_exact_match() && self.master_pic && self.lapic
    }
}

fn restore_controller_components_with<E, P, L>(mut restore_pic: P, mut restore_lapic: L) -> Result<(), E>
where
    P: FnMut() -> Result<(), E>,
    L: FnMut() -> Result<(), E>,
{
    restore_pic()?;
    restore_lapic()?;
    Ok(())
}

#[cfg(test)]
mod controller_checkpoint_tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn controller_restore_orders_master_pic_before_lapic_and_stops_on_failure() {
        let sequence = RefCell::new(Vec::new());
        restore_controller_components_with(
            || {
                sequence.borrow_mut().push("pic");
                Ok::<_, &'static str>(())
            },
            || {
                sequence.borrow_mut().push("lapic");
                Ok::<_, &'static str>(())
            },
        )
        .unwrap();
        assert_eq!(&*sequence.borrow(), &["pic", "lapic"]);

        sequence.borrow_mut().clear();
        let error = restore_controller_components_with(
            || {
                sequence.borrow_mut().push("pic");
                Err::<(), _>("pic failure")
            },
            || {
                sequence.borrow_mut().push("lapic");
                Ok::<_, &'static str>(())
            },
        )
        .unwrap_err();
        assert_eq!(error, "pic failure");
        assert_eq!(&*sequence.borrow(), &["pic"]);
    }
}
