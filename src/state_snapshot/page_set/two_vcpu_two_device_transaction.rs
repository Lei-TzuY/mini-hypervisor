use super::{
    canonical_two_virtio_blk_bars, capture_required_device, page_set_error, verify_required_device,
};
use crate::error::Error;
use crate::kvm::msr::GuestMsrAccessPolicy;
use crate::kvm::sys::{
    HostRegistrationPairCheckpoint, HostRegistrationSpecPair, ReconstructedHostRegistrationPair,
};
use crate::memory::GuestPhysAddr;
use crate::mmio::MmioBus;
use crate::portio::pci::virtio_blk::VirtioBlkDevice;
use crate::state_snapshot::{
    BoundedTwoVcpuFullControllerCheckpoint, BoundedTwoVcpuFullControllerCheckpointComparison,
};
use crate::vcpu::Vcpu;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint {
    controller: BoundedTwoVcpuFullControllerCheckpoint,
    devices: [(u64, VirtioBlkDevice); 2],
}

impl BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint {
    pub fn capture(
        first: &Vcpu,
        second: &Vcpu,
        vm: &crate::kvm::Vm,
        msr_policy: &GuestMsrAccessPolicy,
        mmio: &MmioBus,
        bars: [u64; 2],
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let bars = canonical_two_virtio_blk_bars(bars)?;
        let first_device = capture_required_device(mmio, bars[0], "first two-vCPU transaction")?;
        let second_device = capture_required_device(mmio, bars[1], "second two-vCPU transaction")?;
        let controller = BoundedTwoVcpuFullControllerCheckpoint::capture(
            first,
            second,
            vm,
            msr_policy,
            page_addresses,
        )?;
        Ok(Self {
            controller,
            devices: [(bars[0], first_device), (bars[1], second_device)],
        })
    }

    #[must_use]
    pub const fn controller(&self) -> &BoundedTwoVcpuFullControllerCheckpoint {
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
        first: &Vcpu,
        second: &Vcpu,
        vm: &crate::kvm::Vm,
        mmio: &MmioBus,
    ) -> Result<BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison, Error> {
        let controller = self.controller.verify(first, second, vm)?;
        let device_exact = [
            verify_required_device(mmio, self.devices[0].0, &self.devices[0].1)?,
            verify_required_device(mmio, self.devices[1].0, &self.devices[1].1)?,
        ];
        Ok(
            BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison {
                controller,
                bars: self.device_bars(),
                device_exact,
            },
        )
    }

    pub fn restore_and_verify(
        &self,
        first: &mut Vcpu,
        second: &mut Vcpu,
        vm: &mut crate::kvm::Vm,
        mmio: &mut MmioBus,
    ) -> Result<BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison, Error> {
        capture_required_device(mmio, self.devices[0].0, "first live two-vCPU preflight")?;
        capture_required_device(mmio, self.devices[1].0, "second live two-vCPU preflight")?;

        let controller = self.controller.restore_and_verify(first, second, vm)?;
        require_two_vcpu_controller_exact_before_devices(controller.is_exact_match())?;

        mmio.restore_two_virtio_blk_checkpoints_atomic([
            (self.devices[0].0, &self.devices[0].1),
            (self.devices[1].0, &self.devices[1].1),
        ])?;

        self.verify(first, second, vm, mmio)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison {
    controller: BoundedTwoVcpuFullControllerCheckpointComparison,
    bars: [u64; 2],
    device_exact: [bool; 2],
}

impl BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison {
    #[must_use]
    pub const fn controller(&self) -> &BoundedTwoVcpuFullControllerCheckpointComparison {
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

struct TwoVcpuTwoDeviceCaptureContext<'a> {
    first: &'a Vcpu,
    second: &'a Vcpu,
    vm: &'a crate::kvm::Vm,
    msr_policy: &'a GuestMsrAccessPolicy,
    mmio: &'a MmioBus,
    bars: [u64; 2],
    page_addresses: &'a [GuestPhysAddr],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TwoVcpuTwoDeviceCheckpointTransaction {
    checkpoint: BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint,
    registrations: HostRegistrationPairCheckpoint,
}

impl TwoVcpuTwoDeviceCheckpointTransaction {
    fn capture(
        context: TwoVcpuTwoDeviceCaptureContext<'_>,
        pair: HostRegistrationSpecPair,
        registrations: &ReconstructedHostRegistrationPair,
    ) -> Result<Self, Error> {
        let bars = canonical_two_virtio_blk_bars(context.bars)?;
        require_registration_pair_matches_devices(pair, bars)?;
        registrations.require_checkpoint_quiescent()?;
        let checkpoint = BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint::capture(
            context.first,
            context.second,
            context.vm,
            context.msr_policy,
            context.mmio,
            bars,
            context.page_addresses,
        )?;
        Ok(Self {
            checkpoint,
            registrations: HostRegistrationPairCheckpoint::capture(pair),
        })
    }

    #[must_use]
    const fn checkpoint(&self) -> &BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint {
        &self.checkpoint
    }

    fn restore_and_reconstruct(
        &self,
        backend: &crate::kvm::KvmBackend,
        first: &mut Vcpu,
        second: &mut Vcpu,
        vm: &mut crate::kvm::Vm,
        mmio: &mut MmioBus,
    ) -> Result<
        (
            BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison,
            ReconstructedHostRegistrationPair,
        ),
        Error,
    > {
        let restored = self
            .checkpoint
            .restore_and_verify(first, second, vm, mmio)?;
        if !restored.is_exact_match() {
            return Err(page_set_error(
                "two-vCPU two-device transaction restore",
                "checkpoint did not verify exactly; host registrations were not reconstructed",
            ));
        }
        let registrations = self.registrations.reconstruct(backend, vm)?;
        if let Err(error) = registrations.require_checkpoint_quiescent() {
            let cleanup = registrations.deassign(vm);
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(page_set_error(
                    "two-vCPU two-device registration reconstruction cleanup",
                    format!(
                        "reconstructed pair was not quiescent: {error}; cleanup also failed: {cleanup_error}"
                    ),
                )),
            };
        }

        let post_reconstruction = match self.checkpoint.verify(first, second, vm, mmio) {
            Ok(comparison) => comparison,
            Err(error) => {
                let cleanup = registrations.deassign(vm);
                return match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup_error) => Err(page_set_error(
                        "two-vCPU two-device post-registration verification cleanup",
                        format!(
                            "post-registration verification failed: {error}; cleanup also failed: {cleanup_error}"
                        ),
                    )),
                };
            }
        };
        if !post_reconstruction.is_exact_match() {
            let cleanup = registrations.deassign(vm);
            return match cleanup {
                Ok(()) => Err(page_set_error(
                    "two-vCPU two-device post-registration verification",
                    "reconstructing the host-registration pair changed restored checkpoint state",
                )),
                Err(cleanup_error) => Err(page_set_error(
                    "two-vCPU two-device post-registration cleanup",
                    format!(
                        "registration reconstruction changed restored state; cleanup also failed: {cleanup_error}"
                    ),
                )),
            };
        }
        Ok((post_reconstruction, registrations))
    }
}

fn require_registration_pair_matches_devices(
    pair: HostRegistrationSpecPair,
    bars: [u64; 2],
) -> Result<(), Error> {
    let specs = pair.specs();
    let expected = [
        bars[0].checked_add(0x100).ok_or_else(|| {
            page_set_error(
                "two-vCPU two-device transaction registration binding",
                "first device notify address overflowed",
            )
        })?,
        bars[1].checked_add(0x100).ok_or_else(|| {
            page_set_error(
                "two-vCPU two-device transaction registration binding",
                "second device notify address overflowed",
            )
        })?,
    ];
    let observed = [specs[0].doorbell_address(), specs[1].doorbell_address()];
    if observed != expected {
        return Err(page_set_error(
            "two-vCPU two-device transaction registration binding",
            format!(
                "registration doorbells {observed:?} do not match device notify addresses {expected:?}"
            ),
        ));
    }
    if specs
        .iter()
        .any(|spec| spec.doorbell_length() != 2 || spec.doorbell_datamatch() != 0)
    {
        return Err(page_set_error(
            "two-vCPU two-device transaction registration binding",
            "each fixed virtio-blk registration must use a 2-byte datamatch-zero queue notify",
        ));
    }
    Ok(())
}

fn require_two_vcpu_controller_exact_before_devices(exact: bool) -> Result<(), Error> {
    if !exact {
        return Err(page_set_error(
            "two-vCPU two-device controller restore verification",
            "page/vCPU/MP/controller/LAPIC state was not exact; neither device was restored",
        ));
    }
    Ok(())
}

include!("two_vcpu_two_device_transaction_schema_v1.rs");

#[cfg(test)]
mod two_vcpu_two_device_transaction_tests {
    use super::*;
    use crate::kvm::sys::{
        default_two_host_registration_pair, HostRegistrationSpec, TWO_HOST_REGISTRATION_FIRST_BAR,
        TWO_HOST_REGISTRATION_SECOND_BAR,
    };
    use crate::portio::pci::virtio_blk::VIRTIO_BLK_BAR_SIZE;

    #[test]
    fn default_registration_pair_is_bound_to_the_two_device_bars() {
        require_registration_pair_matches_devices(
            default_two_host_registration_pair().unwrap(),
            [
                TWO_HOST_REGISTRATION_FIRST_BAR,
                TWO_HOST_REGISTRATION_SECOND_BAR,
            ],
        )
        .unwrap();

        let wrong = HostRegistrationSpecPair::new([
            HostRegistrationSpec::new(TWO_HOST_REGISTRATION_FIRST_BAR + 0x200, 2, 0, 0).unwrap(),
            HostRegistrationSpec::new(TWO_HOST_REGISTRATION_SECOND_BAR + 0x200, 2, 0, 1).unwrap(),
        ])
        .unwrap();
        assert!(require_registration_pair_matches_devices(
            wrong,
            [
                TWO_HOST_REGISTRATION_FIRST_BAR,
                TWO_HOST_REGISTRATION_SECOND_BAR,
            ],
        )
        .is_err());
    }

    #[test]
    fn device_restore_requires_exact_two_vcpu_controller_state() {
        assert!(require_two_vcpu_controller_exact_before_devices(false).is_err());
        require_two_vcpu_controller_exact_before_devices(true).unwrap();
    }

    #[test]
    fn two_device_bar_pair_remains_canonical_distinct_and_aligned() {
        let first = 0x1000_0000;
        let second = first + u64::from(VIRTIO_BLK_BAR_SIZE);
        assert_eq!(
            canonical_two_virtio_blk_bars([second, first]).unwrap(),
            [first, second]
        );
    }
}

mod two_vcpu_two_device_transaction_guest {
    use super::*;
    include!("two_vcpu_two_device_transaction_guest.rs");
}
pub use two_vcpu_two_device_transaction_guest::*;
