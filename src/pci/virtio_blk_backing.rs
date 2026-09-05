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

        let start = sector.checked_mul(VIRTIO_BLK_SECTOR_SIZE as u64).ok_or(
            VirtioBlkError::RequestRangeOutOfRange {
                sector,
                data_length,
                capacity: VIRTIO_BLK_CAPACITY_SECTORS,
            },
        )?;
        let end = start.checked_add(u64::from(data_length)).ok_or(
            VirtioBlkError::RequestRangeOutOfRange {
                sector,
                data_length,
                capacity: VIRTIO_BLK_CAPACITY_SECTORS,
            },
        )?;
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

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: u64 = 0x1000_0000;
    const DESC: u64 = 0x18000;
    const AVAIL: u64 = 0x18100;
    const USED: u64 = 0x18200;
    const HEADER: u64 = 0x18300;
    const DATA: u64 = 0x18400;
    const STATUS: u64 = 0x18a00;
    const MEMORY_SIZE: u64 = 0x20_000;
    const TWO_SECTORS: u32 = (2 * VIRTIO_BLK_SECTOR_SIZE) as u32;

    fn ready_device() -> VirtioBlkDevice {
        let mut device = VirtioBlkDevice::new(BAR);
        device.driver_features = VIRTIO_F_VERSION_1;
        device.status = VIRTIO_STATUS_ACKNOWLEDGE
            | VIRTIO_STATUS_DRIVER
            | VIRTIO_STATUS_FEATURES_OK
            | VIRTIO_STATUS_DRIVER_OK;
        device.queue_size = 4;
        device.queue_enabled = true;
        device.queue_desc = DESC;
        device.queue_driver = AVAIL;
        device.queue_device = USED;
        device
    }

    fn write_descriptor(
        memory: &mut GuestMemory,
        index: u16,
        address: u64,
        length: u32,
        flags: u16,
        next: u16,
    ) {
        let mut descriptor = [0_u8; 16];
        descriptor[0..8].copy_from_slice(&address.to_le_bytes());
        descriptor[8..12].copy_from_slice(&length.to_le_bytes());
        descriptor[12..14].copy_from_slice(&flags.to_le_bytes());
        descriptor[14..16].copy_from_slice(&next.to_le_bytes());
        memory
            .write(
                GuestPhysAddr::new(DESC + 16 * u64::from(index)),
                &descriptor,
            )
            .unwrap();
    }

    fn prepare_request(
        memory: &mut GuestMemory,
        device: &mut VirtioBlkDevice,
        request_type: u32,
        sector: u64,
        data_length: u32,
        data_flags: u16,
        avail_idx: u16,
        ring_slot: u16,
    ) {
        write_descriptor(memory, 0, HEADER, 16, VIRTQ_DESC_F_NEXT, 1);
        write_descriptor(memory, 1, DATA, data_length, data_flags, 2);
        write_descriptor(memory, 2, STATUS, 1, VIRTQ_DESC_F_WRITE, 0);

        let mut header = [0_u8; 16];
        header[0..4].copy_from_slice(&request_type.to_le_bytes());
        header[8..16].copy_from_slice(&sector.to_le_bytes());
        memory.write(GuestPhysAddr::new(HEADER), &header).unwrap();
        memory
            .write(GuestPhysAddr::new(AVAIL + 2), &avail_idx.to_le_bytes())
            .unwrap();
        memory
            .write(
                GuestPhysAddr::new(AVAIL + 4 + 2 * u64::from(ring_slot)),
                &0_u16.to_le_bytes(),
            )
            .unwrap();
        memory.write(GuestPhysAddr::new(STATUS), &[0xff]).unwrap();
        device.notify_pending = true;
    }

    fn two_sector_payload() -> Vec<u8> {
        (0..TWO_SECTORS as usize)
            .map(|index| (index as u8).wrapping_mul(37).wrapping_add(11))
            .collect()
    }

    fn assert_request_state_unchanged(
        device: &VirtioBlkDevice,
        memory: &GuestMemory,
        original_backing: &[u8],
    ) {
        assert_eq!(device.backing_bytes(), original_backing);
        let mut status = [0_u8; 1];
        memory
            .read(GuestPhysAddr::new(STATUS), &mut status)
            .unwrap();
        assert_eq!(status, [0xff]);
        assert_eq!(read_guest_u16(memory, USED + 2).unwrap(), 0);
        assert_eq!(device.last_avail_idx, 0);
        assert_eq!(device.last_used_idx, 0);
        assert!(device.notify_pending);
        assert_eq!(device.isr_status, 0);
    }

    #[test]
    fn two_sector_atomic_out_then_in_round_trips_across_sector_boundary() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MEMORY_SIZE).unwrap();
        let mut device = ready_device();
        let sector0 = *device.sector0();
        let sector3_before = device.backing_range(3, VIRTIO_BLK_SECTOR_SIZE as u32).unwrap().to_vec();
        let payload = two_sector_payload();
        memory.write(GuestPhysAddr::new(DATA), &payload).unwrap();
        prepare_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_OUT,
            1,
            TWO_SECTORS,
            VIRTQ_DESC_F_NEXT,
            1,
            0,
        );

        let write = device.process_notified_queue_atomic(&mut memory).unwrap();
        assert_eq!(write.sector(), 1);
        assert_eq!(write.length(), 1);
        assert_eq!(device.backing_range(1, TWO_SECTORS).unwrap(), payload);
        assert_eq!(device.sector0(), &sector0);
        assert_eq!(
            device.backing_range(3, VIRTIO_BLK_SECTOR_SIZE as u32).unwrap(),
            sector3_before
        );

        memory
            .write(GuestPhysAddr::new(DATA), &vec![0x5a; TWO_SECTORS as usize])
            .unwrap();
        prepare_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_IN,
            1,
            TWO_SECTORS,
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
            2,
            1,
        );
        let read = device.process_notified_queue_atomic(&mut memory).unwrap();
        assert_eq!(read.sector(), 1);
        assert_eq!(read.length(), TWO_SECTORS + 1);
        let mut readback = vec![0_u8; TWO_SECTORS as usize];
        memory.read(GuestPhysAddr::new(DATA), &mut readback).unwrap();
        assert_eq!(readback, payload);
        assert_eq!(read_guest_u16(&memory, USED + 2).unwrap(), 2);
    }

    #[test]
    fn range_overflow_fails_before_any_device_or_guest_mutation() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MEMORY_SIZE).unwrap();
        let mut device = ready_device();
        let original_backing = device.backing_bytes().to_vec();
        memory
            .write(GuestPhysAddr::new(DATA), &two_sector_payload())
            .unwrap();
        prepare_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_OUT,
            3,
            TWO_SECTORS,
            VIRTQ_DESC_F_NEXT,
            1,
            0,
        );

        let error = device.process_notified_queue_atomic(&mut memory).unwrap_err();
        assert!(matches!(
            error,
            VirtioBlkProcessError::Device(VirtioBlkError::RequestRangeOutOfRange {
                sector: 3,
                data_length: TWO_SECTORS,
                capacity: VIRTIO_BLK_CAPACITY_SECTORS,
            })
        ));
        assert_request_state_unchanged(&device, &memory, &original_backing);
    }

    #[test]
    fn non_sector_multiple_length_fails_before_any_mutation() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MEMORY_SIZE).unwrap();
        let mut device = ready_device();
        let original_backing = device.backing_bytes().to_vec();
        memory
            .write(GuestPhysAddr::new(DATA), &vec![0x33; 513])
            .unwrap();
        prepare_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_OUT,
            1,
            513,
            VIRTQ_DESC_F_NEXT,
            1,
            0,
        );

        let error = device.process_notified_queue_atomic(&mut memory).unwrap_err();
        assert!(matches!(
            error,
            VirtioBlkProcessError::Device(VirtioBlkError::InvalidDataLength {
                length: 513,
                sector_size,
            }) if sector_size == VIRTIO_BLK_SECTOR_SIZE as u32
        ));
        assert_request_state_unchanged(&device, &memory, &original_backing);
    }

    #[test]
    fn legacy_read_processor_uses_requested_sector_from_expanded_backing() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MEMORY_SIZE).unwrap();
        let mut device = ready_device();
        let marker = vec![0xc7; VIRTIO_BLK_SECTOR_SIZE];
        let range = device
            .request_backing_range(2, VIRTIO_BLK_SECTOR_SIZE as u32)
            .unwrap();
        device.backing[range].copy_from_slice(&marker);
        prepare_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_IN,
            2,
            VIRTIO_BLK_SECTOR_SIZE as u32,
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
            1,
            0,
        );

        let completion = device.process_notified_queue(&mut memory).unwrap();
        assert_eq!(completion.sector(), 2);
        assert_eq!(completion.length(), (VIRTIO_BLK_SECTOR_SIZE + 1) as u32);
        let mut data = vec![0_u8; VIRTIO_BLK_SECTOR_SIZE];
        memory.read(GuestPhysAddr::new(DATA), &mut data).unwrap();
        assert_eq!(data, marker);
    }
}
