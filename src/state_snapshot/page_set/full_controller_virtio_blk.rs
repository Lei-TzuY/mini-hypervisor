#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedFullControllerVirtioBlkCheckpoint {
    controller: BoundedFullControllerCheckpoint,
    bar0: u64,
    device: crate::portio::pci::virtio_blk::VirtioBlkDevice,
}

impl BoundedFullControllerVirtioBlkCheckpoint {
    pub fn capture(
        vcpu: &Vcpu,
        vm: &crate::kvm::Vm,
        msr_policy: &GuestMsrAccessPolicy,
        mmio: &crate::mmio::MmioBus,
        bar0: u64,
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let controller =
            BoundedFullControllerCheckpoint::capture(vcpu, vm, msr_policy, page_addresses)?;
        let device = mmio
            .capture_virtio_blk_checkpoint_at(bar0)?
            .ok_or_else(|| {
                page_set_error(
                    "full-controller virtio-blk checkpoint capture",
                    format!("no quiescent virtio-blk device is registered at BAR {bar0:#x}"),
                )
            })?;
        Ok(Self {
            controller,
            bar0,
            device,
        })
    }

    #[must_use]
    pub const fn controller(&self) -> &BoundedFullControllerCheckpoint {
        &self.controller
    }

    #[must_use]
    pub const fn bar0(&self) -> u64 {
        self.bar0
    }

    #[must_use]
    pub const fn device(&self) -> &crate::portio::pci::virtio_blk::VirtioBlkDevice {
        &self.device
    }

    pub fn verify(
        &self,
        vcpu: &Vcpu,
        vm: &crate::kvm::Vm,
        mmio: &crate::mmio::MmioBus,
    ) -> Result<BoundedFullControllerVirtioBlkCheckpointComparison, Error> {
        let controller = self.controller.verify(vcpu, vm)?;
        let device = mmio
            .verify_virtio_blk_checkpoint_at(self.bar0, &self.device)?
            .unwrap_or(false);
        Ok(BoundedFullControllerVirtioBlkCheckpointComparison { controller, device })
    }

    pub fn restore_and_verify(
        &self,
        vcpu: &Vcpu,
        vm: &mut crate::kvm::Vm,
        mmio: &mut crate::mmio::MmioBus,
    ) -> Result<BoundedFullControllerVirtioBlkCheckpointComparison, Error> {
        let controller = self.controller.restore_and_verify(vcpu, vm)?;
        restore_device_only_after_controller_exact(&controller, || {
            mmio.restore_virtio_blk_checkpoint_at(self.bar0, &self.device)?
                .ok_or_else(|| {
                    page_set_error(
                        "full-controller virtio-blk checkpoint device restore",
                        format!("virtio-blk device at BAR {:#x} disappeared", self.bar0),
                    )
                })?;
            Ok(())
        })?;
        self.verify(vcpu, vm, mmio)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedFullControllerVirtioBlkCheckpointComparison {
    controller: BoundedFullControllerCheckpointComparison,
    device: bool,
}

impl BoundedFullControllerVirtioBlkCheckpointComparison {
    #[must_use]
    pub const fn controller(&self) -> &BoundedFullControllerCheckpointComparison {
        &self.controller
    }

    #[must_use]
    pub const fn device_exact(&self) -> bool {
        self.device
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.controller.is_exact_match() && self.device
    }
}

fn restore_device_only_after_controller_exact(
    controller: &BoundedFullControllerCheckpointComparison,
    restore_device: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    if !controller.is_exact_match() {
        return Err(page_set_error(
            "full-controller virtio-blk checkpoint controller restore verification",
            "page/VCPU/controller state was not exact after restore; device state was not mutated",
        ));
    }
    restore_device()
}

#[cfg(test)]
mod full_controller_virtio_blk_tests {
    use super::*;
    use std::cell::Cell;

    fn comparison_with_exactness(exact: bool) -> BoundedFullControllerCheckpointComparison {
        let page = GuestPhysAddr::new(0x30000);
        BoundedFullControllerCheckpointComparison {
            base: BoundedControllerCheckpointComparison {
                guest: BoundedPageSetCheckpointComparison {
                    pages: vec![BoundedCheckpointPageComparison {
                        address: page,
                        exact,
                    }],
                    vcpu: VcpuStateSnapshotComparison {
                        registers: exact,
                        special_registers: exact,
                        msrs: exact,
                    },
                },
                master_pic: exact,
                lapic: exact,
            },
            slave_pic: exact,
            ioapic: exact,
        }
    }

    #[test]
    fn device_restore_is_blocked_until_full_controller_restore_is_exact() {
        let called = Cell::new(false);
        let mismatch = comparison_with_exactness(false);
        assert!(restore_device_only_after_controller_exact(&mismatch, || {
            called.set(true);
            Ok(())
        })
        .is_err());
        assert!(!called.get());

        let exact = comparison_with_exactness(true);
        restore_device_only_after_controller_exact(&exact, || {
            called.set(true);
            Ok(())
        })
        .unwrap();
        assert!(called.get());
    }
}
