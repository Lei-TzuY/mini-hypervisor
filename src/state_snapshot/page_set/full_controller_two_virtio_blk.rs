use crate::kvm::sys::ReconstructedHostRegistrationPair;
use crate::mmio::MmioBus;
use crate::portio::pci::virtio_blk::{VirtioBlkDevice, VIRTIO_BLK_BAR_SIZE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedFullControllerTwoVirtioBlkCheckpoint {
    controller: BoundedFullControllerCheckpoint,
    devices: [(u64, VirtioBlkDevice); 2],
}

impl BoundedFullControllerTwoVirtioBlkCheckpoint {
    pub fn capture(
        vcpu: &Vcpu,
        vm: &crate::kvm::Vm,
        msr_policy: &GuestMsrAccessPolicy,
        mmio: &MmioBus,
        bars: [u64; 2],
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let bars = canonical_two_virtio_blk_bars(bars)?;
        let first = capture_required_device(mmio, bars[0], "first")?;
        let second = capture_required_device(mmio, bars[1], "second")?;
        let controller =
            BoundedFullControllerCheckpoint::capture(vcpu, vm, msr_policy, page_addresses)?;
        Ok(Self {
            controller,
            devices: [(bars[0], first), (bars[1], second)],
        })
    }

    pub(crate) fn capture_with_acceleration_quiescence(
        vcpu: &Vcpu,
        vm: &crate::kvm::Vm,
        msr_policy: &GuestMsrAccessPolicy,
        mmio: &MmioBus,
        bars: [u64; 2],
        page_addresses: &[GuestPhysAddr],
        registrations: &ReconstructedHostRegistrationPair,
    ) -> Result<Self, Error> {
        // The caller must already have stopped guest execution. Polling the ioeventfds is
        // intentionally non-consuming, so a rejected capture leaves every pending doorbell
        // available to the normal queue-service path.
        registrations.require_checkpoint_quiescent()?;
        Self::capture(vcpu, vm, msr_policy, mmio, bars, page_addresses)
    }

    #[must_use]
    pub const fn controller(&self) -> &BoundedFullControllerCheckpoint {
        &self.controller
    }

    #[must_use]
    pub const fn device_bars(&self) -> [u64; 2] {
        [self.devices[0].0, self.devices[1].0]
    }

    #[must_use]
    pub fn device(&self, bar: u64) -> Option<&VirtioBlkDevice> {
        self.devices
            .iter()
            .find(|(address, _)| *address == bar)
            .map(|(_, device)| device)
    }

    pub fn verify(
        &self,
        vcpu: &Vcpu,
        vm: &crate::kvm::Vm,
        mmio: &MmioBus,
    ) -> Result<BoundedFullControllerTwoVirtioBlkCheckpointComparison, Error> {
        let controller = self.controller.verify(vcpu, vm)?;
        let device_exact = [
            verify_required_device(mmio, self.devices[0].0, &self.devices[0].1)?,
            verify_required_device(mmio, self.devices[1].0, &self.devices[1].1)?,
        ];
        Ok(BoundedFullControllerTwoVirtioBlkCheckpointComparison {
            controller,
            bars: self.device_bars(),
            device_exact,
        })
    }

    pub fn restore_and_verify(
        &self,
        vcpu: &Vcpu,
        vm: &mut crate::kvm::Vm,
        mmio: &mut MmioBus,
    ) -> Result<BoundedFullControllerTwoVirtioBlkCheckpointComparison, Error> {
        // Preflight the entire device set before mutating page/VCPU/controller or either device.
        // This keeps a missing or in-flight second device from producing a partial restore.
        capture_required_device(mmio, self.devices[0].0, "first live preflight")?;
        capture_required_device(mmio, self.devices[1].0, "second live preflight")?;

        let controller = self.controller.restore_and_verify(vcpu, vm)?;
        require_controller_exact_before_two_devices(controller.is_exact_match())?;

        mmio.restore_two_virtio_blk_checkpoints_atomic([
            (self.devices[0].0, &self.devices[0].1),
            (self.devices[1].0, &self.devices[1].1),
        ])?;

        self.verify(vcpu, vm, mmio)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedFullControllerTwoVirtioBlkCheckpointComparison {
    controller: BoundedFullControllerCheckpointComparison,
    bars: [u64; 2],
    device_exact: [bool; 2],
}

impl BoundedFullControllerTwoVirtioBlkCheckpointComparison {
    #[must_use]
    pub const fn controller(&self) -> &BoundedFullControllerCheckpointComparison {
        &self.controller
    }

    #[must_use]
    pub fn device_exact(&self, bar: u64) -> Option<bool> {
        self.bars
            .iter()
            .position(|address| *address == bar)
            .map(|index| self.device_exact[index])
    }

    #[must_use]
    pub const fn all_devices_exact(&self) -> bool {
        self.device_exact[0] && self.device_exact[1]
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.controller.is_exact_match() && self.all_devices_exact()
    }
}

fn canonical_two_virtio_blk_bars(mut bars: [u64; 2]) -> Result<[u64; 2], Error> {
    bars.sort_unstable();
    if bars[0] == bars[1] {
        return Err(page_set_error(
            "two virtio-blk checkpoint BAR ownership",
            format!("duplicate virtio-blk BAR {:#x}", bars[0]),
        ));
    }
    for bar in bars {
        if bar % u64::from(VIRTIO_BLK_BAR_SIZE) != 0 {
            return Err(page_set_error(
                "two virtio-blk checkpoint BAR ownership",
                format!(
                    "virtio-blk BAR {bar:#x} is not {:#x}-aligned",
                    VIRTIO_BLK_BAR_SIZE
                ),
            ));
        }
    }
    Ok(bars)
}

fn capture_required_device(
    mmio: &MmioBus,
    bar: u64,
    role: &'static str,
) -> Result<VirtioBlkDevice, Error> {
    mmio.capture_virtio_blk_checkpoint_at(bar)?.ok_or_else(|| {
        page_set_error(
            "two virtio-blk checkpoint device preflight",
            format!("{role} virtio-blk device at BAR {bar:#x} is missing"),
        )
    })
}

fn verify_required_device(
    mmio: &MmioBus,
    bar: u64,
    expected: &VirtioBlkDevice,
) -> Result<bool, Error> {
    Ok(mmio
        .verify_virtio_blk_checkpoint_at(bar, expected)?
        .unwrap_or(false))
}

fn require_controller_exact_before_two_devices(controller_exact: bool) -> Result<(), Error> {
    if !controller_exact {
        return Err(page_set_error(
            "two virtio-blk checkpoint controller restore verification",
            "page/VCPU/controller state was not exact after restore; neither device was restored",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod full_controller_two_virtio_blk_tests {
    use super::*;

    const FIRST_BAR: u64 = 0x1000_0000;
    const SECOND_BAR: u64 = FIRST_BAR + VIRTIO_BLK_BAR_SIZE as u64;

    #[test]
    fn bar_pair_is_canonical_distinct_and_aligned() {
        assert_eq!(
            canonical_two_virtio_blk_bars([SECOND_BAR, FIRST_BAR]).unwrap(),
            [FIRST_BAR, SECOND_BAR]
        );
        assert!(canonical_two_virtio_blk_bars([FIRST_BAR, FIRST_BAR]).is_err());
        assert!(canonical_two_virtio_blk_bars([FIRST_BAR + 1, SECOND_BAR]).is_err());
    }

    #[test]
    fn controller_must_be_exact_before_device_restore() {
        assert!(require_controller_exact_before_two_devices(false).is_err());
        require_controller_exact_before_two_devices(true).unwrap();
    }

    #[test]
    fn atomic_restore_preflights_both_devices_before_mutating_first() {
        let mut bus = MmioBus::empty();
        bus.register_virtio_blk_device_at(FIRST_BAR).unwrap();
        let before = bus
            .capture_virtio_blk_checkpoint_at(FIRST_BAR)
            .unwrap()
            .unwrap();

        let mut first_snapshot = VirtioBlkDevice::new(FIRST_BAR);
        first_snapshot.write(0x14, &[1]).unwrap();
        let second_snapshot = VirtioBlkDevice::new(SECOND_BAR);

        assert!(bus
            .restore_two_virtio_blk_checkpoints_atomic([
                (FIRST_BAR, &first_snapshot),
                (SECOND_BAR, &second_snapshot),
            ])
            .is_err());
        assert_eq!(
            bus.capture_virtio_blk_checkpoint_at(FIRST_BAR)
                .unwrap()
                .unwrap(),
            before
        );
    }
}

mod full_controller_two_virtio_blk_guest {
    use super::*;
    include!("full_controller_two_virtio_blk_guest.rs");
}
pub use full_controller_two_virtio_blk_guest::*;
