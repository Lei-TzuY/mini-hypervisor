use crate::memory::GuestMemory;
use crate::portio::pci::virtio_blk::{
    VirtioBlkProcessError, VirtioBlkQueueCompletion, VIRTIO_BLK_SECTOR_SIZE,
};

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
}
