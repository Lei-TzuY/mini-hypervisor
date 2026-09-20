use crate::error::{Error, HostEnvironmentError};
use crate::memory::GuestMemory;
use crate::portio::pci::virtio_blk::{
    VirtioBlkDevice, VirtioBlkProcessError, VirtioBlkQueueCompletion, VIRTIO_BLK_SECTOR_SIZE,
};
use std::io;

impl super::MmioBus {
    pub fn process_virtio_blk_notification_atomic(
        &mut self,
        address: u64,
        memory: &mut GuestMemory,
    ) -> Result<Option<VirtioBlkQueueCompletion>, VirtioBlkProcessError> {
        match self
            .virtio_blk_devices
            .iter_mut()
            .find(|device| device.bar0() == address)
        {
            Some(device) => device.process_notified_queue_atomic(memory).map(Some),
            None => Ok(None),
        }
    }

    #[must_use]
    pub fn virtio_blk_sector_at(&self, address: u64) -> Option<&[u8; VIRTIO_BLK_SECTOR_SIZE]> {
        self.virtio_blk_devices
            .iter()
            .find(|device| device.bar0() == address)
            .map(|device| device.sector0())
    }

    #[must_use]
    pub fn virtio_blk_backing_range_at(
        &self,
        address: u64,
        sector: u64,
        data_length: u32,
    ) -> Option<&[u8]> {
        self.virtio_blk_devices
            .iter()
            .find(|device| device.bar0() == address)
            .and_then(|device| device.backing_range(sector, data_length))
    }

    pub fn capture_virtio_blk_checkpoint_at(
        &self,
        address: u64,
    ) -> Result<Option<VirtioBlkDevice>, Error> {
        let Some(device) = self
            .virtio_blk_devices
            .iter()
            .find(|device| device.bar0() == address)
        else {
            return Ok(None);
        };
        if !device.checkpoint_quiescent() {
            return Err(virtio_blk_checkpoint_error(
                "capture virtio-blk checkpoint state",
                "queue notification is still in flight",
            ));
        }
        Ok(Some(device.clone()))
    }

    pub fn verify_virtio_blk_checkpoint_at(
        &self,
        address: u64,
        snapshot: &VirtioBlkDevice,
    ) -> Result<Option<bool>, Error> {
        if snapshot.bar0() != address {
            return Err(virtio_blk_checkpoint_error(
                "verify virtio-blk checkpoint state",
                format!(
                    "snapshot BAR {:#x} does not match requested BAR {address:#x}",
                    snapshot.bar0()
                ),
            ));
        }
        Ok(self
            .virtio_blk_devices
            .iter()
            .find(|device| device.bar0() == address)
            .map(|device| device == snapshot))
    }

    pub fn restore_virtio_blk_checkpoint_at(
        &mut self,
        address: u64,
        snapshot: &VirtioBlkDevice,
    ) -> Result<Option<()>, Error> {
        if snapshot.bar0() != address || !snapshot.checkpoint_quiescent() {
            return Err(virtio_blk_checkpoint_error(
                "restore virtio-blk checkpoint state",
                "snapshot BAR identity or quiescence contract is invalid",
            ));
        }
        let Some(device) = self
            .virtio_blk_devices
            .iter_mut()
            .find(|device| device.bar0() == address)
        else {
            return Ok(None);
        };
        if !device.checkpoint_quiescent() {
            return Err(virtio_blk_checkpoint_error(
                "restore virtio-blk checkpoint state",
                "live device has an in-flight queue notification",
            ));
        }
        *device = snapshot.clone();
        Ok(Some(()))
    }

    pub fn restore_two_virtio_blk_checkpoints_atomic(
        &mut self,
        checkpoints: [(u64, &VirtioBlkDevice); 2],
    ) -> Result<(), Error> {
        let [(first_address, first_snapshot), (second_address, second_snapshot)] = checkpoints;
        if first_address >= second_address {
            return Err(virtio_blk_checkpoint_error(
                "restore two virtio-blk checkpoint states",
                "two-device checkpoint BARs must be distinct and strictly increasing",
            ));
        }

        for (address, snapshot) in [
            (first_address, first_snapshot),
            (second_address, second_snapshot),
        ] {
            if snapshot.bar0() != address || !snapshot.checkpoint_quiescent() {
                return Err(virtio_blk_checkpoint_error(
                    "restore two virtio-blk checkpoint states",
                    format!(
                        "snapshot for BAR {address:#x} has mismatched identity or is not quiescent"
                    ),
                ));
            }
        }

        let first_index = self
            .virtio_blk_devices
            .iter()
            .position(|device| device.bar0() == first_address)
            .ok_or_else(|| {
                virtio_blk_checkpoint_error(
                    "restore two virtio-blk checkpoint states",
                    format!("live virtio-blk device at BAR {first_address:#x} is missing"),
                )
            })?;
        let second_index = self
            .virtio_blk_devices
            .iter()
            .position(|device| device.bar0() == second_address)
            .ok_or_else(|| {
                virtio_blk_checkpoint_error(
                    "restore two virtio-blk checkpoint states",
                    format!("live virtio-blk device at BAR {second_address:#x} is missing"),
                )
            })?;
        if first_index == second_index {
            return Err(virtio_blk_checkpoint_error(
                "restore two virtio-blk checkpoint states",
                "two checkpoint BARs resolved to the same live device",
            ));
        }
        if !self.virtio_blk_devices[first_index].checkpoint_quiescent()
            || !self.virtio_blk_devices[second_index].checkpoint_quiescent()
        {
            return Err(virtio_blk_checkpoint_error(
                "restore two virtio-blk checkpoint states",
                "all live devices must be quiescent before either device is restored",
            ));
        }

        if first_index < second_index {
            let (before_second, second_and_after) =
                self.virtio_blk_devices.split_at_mut(second_index);
            before_second[first_index] = first_snapshot.clone();
            second_and_after[0] = second_snapshot.clone();
        } else {
            let (before_first, first_and_after) = self.virtio_blk_devices.split_at_mut(first_index);
            before_first[second_index] = second_snapshot.clone();
            first_and_after[0] = first_snapshot.clone();
        }
        Ok(())
    }
}

fn virtio_blk_checkpoint_error(operation: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::Io {
        operation,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}
