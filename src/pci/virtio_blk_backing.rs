use super::*;
use std::ops::Range;

pub const VIRTIO_BLK_CAPACITY_SECTORS: u64 = 4;
pub const VIRTIO_BLK_BACKING_SIZE: usize =
    VIRTIO_BLK_SECTOR_SIZE * VIRTIO_BLK_CAPACITY_SECTORS as usize;

pub(super) fn deterministic_backing() -> [u8; VIRTIO_BLK_BACKING_SIZE] {
    let mut backing = [0_u8; VIRTIO_BLK_BACKING_SIZE];
    let sector0 = deterministic_sector();
    backing[..VIRTIO_BLK_SECTOR_SIZE].copy_from_slice(&sector0);
    for sector in 1..VIRTIO_BLK_CAPACITY_SECTORS as usize {
        let start = sector * VIRTIO_BLK_SECTOR_SIZE;
        let end = start + VIRTIO_BLK_SECTOR_SIZE;
        for (index, byte) in backing[start..end].iter_mut().enumerate() {
            *byte = (index as u8)
                .wrapping_mul(17)
                .wrapping_add(3)
                .wrapping_add((sector as u8).wrapping_mul(41));
        }
    }
    backing
}

impl VirtioBlkDevice {
    pub(super) fn request_backing_range(
        &self,
        sector: u64,
        data_length: u32,
    ) -> Result<Range<usize>, VirtioBlkError> {
        if data_length == 0 || data_length % VIRTIO_BLK_SECTOR_SIZE as u32 != 0 {
            return Err(VirtioBlkError::InvalidDataLength {
                length: data_length,
                sector_size: VIRTIO_BLK_SECTOR_SIZE as u32,
            });
        }
        if sector >= VIRTIO_BLK_CAPACITY_SECTORS {
            return Err(VirtioBlkError::SectorOutOfRange {
                sector,
                capacity: VIRTIO_BLK_CAPACITY_SECTORS,
            });
        }

        let start = sector
            .checked_mul(VIRTIO_BLK_SECTOR_SIZE as u64)
            .ok_or(VirtioBlkError::RequestRangeOutOfRange {
                sector,
                data_length,
                capacity: VIRTIO_BLK_CAPACITY_SECTORS,
            })?;
        let end = start
            .checked_add(u64::from(data_length))
            .ok_or(VirtioBlkError::RequestRangeOutOfRange {
                sector,
                data_length,
                capacity: VIRTIO_BLK_CAPACITY_SECTORS,
            })?;
        if end > VIRTIO_BLK_BACKING_SIZE as u64 {
            return Err(VirtioBlkError::RequestRangeOutOfRange {
                sector,
                data_length,
                capacity: VIRTIO_BLK_CAPACITY_SECTORS,
            });
        }

        let start = usize::try_from(start).expect("bounded virtio-blk backing offset fits usize");
        let end = usize::try_from(end).expect("bounded virtio-blk backing end fits usize");
        Ok(start..end)
    }

    #[must_use]
    pub fn backing_bytes(&self) -> &[u8] {
        &self.backing
    }

    #[must_use]
    pub fn backing_range(&self, sector: u64, data_length: u32) -> Option<&[u8]> {
        let range = self.request_backing_range(sector, data_length).ok()?;
        Some(&self.backing[range])
    }
}
