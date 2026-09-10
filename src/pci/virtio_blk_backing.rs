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

    #[must_use]
    pub const fn checkpoint_quiescent(&self) -> bool {
        !self.notify_pending
    }

    #[must_use]
    pub const fn checkpoint_last_avail_idx(&self) -> u16 {
        self.last_avail_idx
    }

    #[must_use]
    pub const fn checkpoint_last_used_idx(&self) -> u16 {
        self.last_used_idx
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

    #[derive(Clone, Copy)]
    struct RequestSpec {
        request_type: u32,
        sector: u64,
        data_length: u32,
        data_flags: u16,
        avail_idx: u16,
        ring_slot: u16,
    }

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

    fn write_request(memory: &mut GuestMemory, request_type: u32) {
        let mut header = [0_u8; 16];
        header[0..4].copy_from_slice(&request_type.to_le_bytes());
        header[8..16].copy_from_slice(&0_u64.to_le_bytes());
        memory.write(GuestPhysAddr::new(HEADER), &header).unwrap();
    }

    fn prepare_request(
        memory: &mut GuestMemory,
        device: &mut VirtioBlkDevice,
        request_type: u32,
        data_flags: u16,
        avail_idx: u16,
        ring_slot: u16,
    ) {
        write_descriptor(memory, 0, HEADER, 16, VIRTQ_DESC_F_NEXT, 1);
        write_descriptor(
            memory,
            1,
            DATA,
            VIRTIO_BLK_SECTOR_SIZE as u32,
            data_flags,
            2,
        );
        write_descriptor(memory, 2, STATUS, 1, VIRTQ_DESC_F_WRITE, 0);
        write_request(memory, request_type);
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

    fn mutation_sector() -> [u8; VIRTIO_BLK_SECTOR_SIZE] {
        let mut bytes = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(29).wrapping_add(7);
        }
        bytes[..16].copy_from_slice(b"BLK-WRITE-0000!!");
        bytes[VIRTIO_BLK_SECTOR_SIZE - 8..].copy_from_slice(b"WRTBACK!");
        bytes
    }

    #[test]
    fn out_then_in_round_trips_mutated_sector_in_same_device() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MEMORY_SIZE).unwrap();
        let mut device = ready_device(USED);
        let payload = mutation_sector();
        memory.write(GuestPhysAddr::new(DATA), &payload).unwrap();
        prepare_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_OUT,
            VIRTQ_DESC_F_NEXT,
            1,
            0,
        );

        let write_completion = device.process_notified_queue_atomic(&mut memory).unwrap();
        assert_eq!(write_completion.length(), 1);
        assert_eq!(write_completion.sector(), 0);
        assert_eq!(device.sector0(), &payload);
        assert_eq!(read_guest_u16(&memory, USED + 2).unwrap(), 1);
        let mut used0_len = [0_u8; 4];
        memory
            .read(GuestPhysAddr::new(USED + 8), &mut used0_len)
            .unwrap();
        assert_eq!(u32::from_le_bytes(used0_len), 1);

        memory
            .write(GuestPhysAddr::new(DATA), &[0x5a; VIRTIO_BLK_SECTOR_SIZE])
            .unwrap();
        prepare_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_IN,
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
            2,
            1,
        );

        let read_completion = device.process_notified_queue_atomic(&mut memory).unwrap();
        assert_eq!(read_completion.length(), 513);
        assert_eq!(read_guest_u16(&memory, USED + 2).unwrap(), 2);
        let mut used1 = [0_u8; 8];
        memory
            .read(GuestPhysAddr::new(USED + 12), &mut used1)
            .unwrap();
        assert_eq!(u32::from_le_bytes(used1[0..4].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(used1[4..8].try_into().unwrap()), 513);
        let mut readback = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
        memory
            .read(GuestPhysAddr::new(DATA), &mut readback)
            .unwrap();
        assert_eq!(readback, payload);
        let mut status = [0xff_u8];
        memory
            .read(GuestPhysAddr::new(STATUS), &mut status)
            .unwrap();
        assert_eq!(status, [VIRTIO_BLK_S_OK]);
        assert_eq!(device.last_avail_idx, 2);
        assert_eq!(device.last_used_idx, 2);
    }

    #[test]
    fn out_preflight_failure_does_not_mutate_backing_or_guest_completion() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MEMORY_SIZE).unwrap();
        let invalid_used = MEMORY_SIZE - 8;
        let mut device = ready_device(invalid_used);
        let original = *device.sector0();
        let payload = mutation_sector();
        memory.write(GuestPhysAddr::new(DATA), &payload).unwrap();
        prepare_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_OUT,
            VIRTQ_DESC_F_NEXT,
            1,
            0,
        );
        memory
            .write(GuestPhysAddr::new(invalid_used + 2), &0_u16.to_le_bytes())
            .unwrap();

        assert!(device.process_notified_queue_atomic(&mut memory).is_err());
        assert_eq!(device.sector0(), &original);
        let mut status = [0_u8];
        memory
            .read(GuestPhysAddr::new(STATUS), &mut status)
            .unwrap();
        assert_eq!(status, [0xff]);
        assert_eq!(device.last_avail_idx, 0);
        assert_eq!(device.last_used_idx, 0);
        assert!(device.notify_pending);
        assert_eq!(device.isr_status, 0);
    }

    #[test]
    fn in_preflight_failure_does_not_partially_write_data_or_status() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MEMORY_SIZE).unwrap();
        let invalid_used = MEMORY_SIZE - 8;
        let mut device = ready_device(invalid_used);
        memory
            .write(GuestPhysAddr::new(DATA), &[0x5a; VIRTIO_BLK_SECTOR_SIZE])
            .unwrap();
        prepare_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_IN,
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
            1,
            0,
        );
        memory
            .write(GuestPhysAddr::new(invalid_used + 2), &0_u16.to_le_bytes())
            .unwrap();

        assert!(device.process_notified_queue_atomic(&mut memory).is_err());
        let mut data = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
        memory.read(GuestPhysAddr::new(DATA), &mut data).unwrap();
        assert_eq!(data, [0x5a; VIRTIO_BLK_SECTOR_SIZE]);
        let mut status = [0_u8];
        memory
            .read(GuestPhysAddr::new(STATUS), &mut status)
            .unwrap();
        assert_eq!(status, [0xff]);
        assert_eq!(device.last_avail_idx, 0);
        assert_eq!(device.last_used_idx, 0);
        assert!(device.notify_pending);
        assert_eq!(device.isr_status, 0);
    }

    #[test]
    fn checkpoint_introspection_tracks_queue_progress_and_inflight_notify() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MEMORY_SIZE).unwrap();
        let mut device = ready_device(USED);
        assert!(device.checkpoint_quiescent());
        assert_eq!(device.checkpoint_last_avail_idx(), 0);
        assert_eq!(device.checkpoint_last_used_idx(), 0);

        prepare_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_IN,
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
            1,
            0,
        );
        assert!(!device.checkpoint_quiescent());

        device.process_notified_queue_atomic(&mut memory).unwrap();
        assert!(device.checkpoint_quiescent());
        assert_eq!(device.checkpoint_last_avail_idx(), 1);
        assert_eq!(device.checkpoint_last_used_idx(), 1);
    }
}
