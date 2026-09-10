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
}

fn virtio_blk_checkpoint_error(operation: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::Io {
        operation,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}
