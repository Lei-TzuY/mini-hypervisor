pub const VIRTIO_BLK_T_OUT: u32 = 1;

impl VirtioBlkDevice {
    #[must_use]
    pub const fn sector0(&self) -> &[u8; VIRTIO_BLK_SECTOR_SIZE] {
        &self.sector0
    }

    pub fn process_notified_queue_atomic(
        &mut self,
        memory: &mut GuestMemory,
    ) -> Result<VirtioBlkQueueCompletion, VirtioBlkProcessError> {
        self.ensure_queue_ready()?;
        if !self.notify_pending {
            return Err(VirtioBlkError::QueueNotReady.into());
        }

        let avail_idx = read_guest_u16(memory, checked_add(self.queue_driver, 2)?)?;
        let expected_avail = self.last_avail_idx.wrapping_add(1);
        if avail_idx != expected_avail {
            return Err(VirtioBlkError::UnexpectedAvailIndex {
                expected: expected_avail,
                actual: avail_idx,
            }
            .into());
        }
        let slot = self.last_avail_idx % self.queue_size;
        let head = read_guest_u16(
            memory,
            checked_add(self.queue_driver, 4 + 2 * u64::from(slot))?,
        )?;
        self.ensure_descriptor_index(head)?;

        let header = self.read_descriptor(memory, head)?;
        self.require_flags(head, header.flags, VIRTQ_DESC_F_NEXT)?;
        self.require_length(head, header.length, 16)?;
        let data_index = header.next;
        self.ensure_descriptor_index(data_index)?;
        if data_index == head {
            return Err(VirtioBlkError::DescriptorChainCycle { index: data_index }.into());
        }

        let data = self.read_descriptor(memory, data_index)?;
        let status_index = data.next;
        self.ensure_descriptor_index(status_index)?;
        if status_index == head || status_index == data_index {
            return Err(VirtioBlkError::DescriptorChainCycle {
                index: status_index,
            }
            .into());
        }
        let status = self.read_descriptor(memory, status_index)?;
        self.require_flags(status_index, status.flags, VIRTQ_DESC_F_WRITE)?;
        self.require_length(status_index, status.length, 1)?;

        let mut request = [0_u8; 16];
        memory.read(GuestPhysAddr::new(header.address), &mut request)?;
        let request_type = u32::from_le_bytes(request[0..4].try_into().unwrap());
        let reserved = u32::from_le_bytes(request[4..8].try_into().unwrap());
        let sector = u64::from_le_bytes(request[8..16].try_into().unwrap());
        if request_type != VIRTIO_BLK_T_IN && request_type != VIRTIO_BLK_T_OUT {
            return Err(VirtioBlkError::InvalidRequestType { request_type }.into());
        }
        if reserved != 0 {
            return Err(VirtioBlkError::InvalidRequestReserved { reserved }.into());
        }
        if sector >= VIRTIO_BLK_CAPACITY_SECTORS {
            return Err(VirtioBlkError::SectorOutOfRange {
                sector,
                capacity: VIRTIO_BLK_CAPACITY_SECTORS,
            }
            .into());
        }

        let expected_data_flags = if request_type == VIRTIO_BLK_T_IN {
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE
        } else {
            VIRTQ_DESC_F_NEXT
        };
        self.require_flags(data_index, data.flags, expected_data_flags)?;
        self.require_length(data_index, data.length, VIRTIO_BLK_SECTOR_SIZE as u32)?;

        let used_idx_address = checked_add(self.queue_device, 2)?;
        let used_idx = read_guest_u16(memory, used_idx_address)?;
        if used_idx != self.last_used_idx {
            return Err(VirtioBlkError::UnexpectedUsedIndex {
                expected: self.last_used_idx,
                actual: used_idx,
            }
            .into());
        }
        let used_slot = self.last_used_idx % self.queue_size;
        let used_element = checked_add(self.queue_device, 4 + 8 * u64::from(used_slot))?;

        // Complete every fallible read and validate every guest range that will be written before
        // mutating guest output, the backing sector, queue indices, notify state, or ISR state. The
        // private anonymous RAM mapping is stable for the duration of this call, so successful
        // preflight reads prove the subsequent writes fit the same registered region.
        let mut outgoing_sector = None;
        if request_type == VIRTIO_BLK_T_OUT {
            let mut bytes = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
            memory.read(GuestPhysAddr::new(data.address), &mut bytes)?;
            outgoing_sector = Some(bytes);
        } else {
            preflight_guest_output(memory, data.address, VIRTIO_BLK_SECTOR_SIZE)?;
        }
        preflight_guest_output(memory, status.address, 1)?;
        preflight_guest_output(memory, used_element, 8)?;
        preflight_guest_output(memory, used_idx_address, 2)?;

        let written = if request_type == VIRTIO_BLK_T_IN {
            let sector_bytes = self.sector0;
            memory.write(GuestPhysAddr::new(data.address), &sector_bytes)?;
            (VIRTIO_BLK_SECTOR_SIZE + 1) as u32
        } else {
            1
        };
        memory.write(GuestPhysAddr::new(status.address), &[VIRTIO_BLK_S_OK])?;
        let mut element = [0_u8; 8];
        element[0..4].copy_from_slice(&u32::from(head).to_le_bytes());
        element[4..8].copy_from_slice(&written.to_le_bytes());
        memory.write(GuestPhysAddr::new(used_element), &element)?;
        let next_used = self.last_used_idx.wrapping_add(1);
        memory.write(GuestPhysAddr::new(used_idx_address), &next_used.to_le_bytes())?;

        if let Some(bytes) = outgoing_sector {
            self.sector0 = bytes;
        }
        self.last_avail_idx = avail_idx;
        self.last_used_idx = next_used;
        self.notify_pending = false;
        self.isr_status |= VIRTIO_ISR_QUEUE_INTERRUPT;
        Ok(VirtioBlkQueueCompletion {
            descriptor_id: u32::from(head),
            length: written,
            sector,
        })
    }
}

fn preflight_guest_output(
    memory: &GuestMemory,
    address: u64,
    length: usize,
) -> Result<(), Error> {
    let mut scratch = vec![0_u8; length];
    memory.read(GuestPhysAddr::new(address), &mut scratch)
}

#[cfg(test)]
mod write_readback_tests {
    use super::*;

    const BAR: u64 = 0x1000_0000;
    const DESC: u64 = 0x18000;
    const AVAIL: u64 = 0x18100;
    const USED: u64 = 0x18200;
    const HEADER: u64 = 0x18300;
    const DATA: u64 = 0x18400;
    const STATUS: u64 = 0x18600;
    const MEMORY_SIZE: u64 = 0x20_000;

    fn ready_device(used: u64) -> VirtioBlkDevice {
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
        device.queue_device = used;
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
            .write(
                GuestPhysAddr::new(DATA),
                &[0x5a; VIRTIO_BLK_SECTOR_SIZE],
            )
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
        memory.read(GuestPhysAddr::new(DATA), &mut readback).unwrap();
        assert_eq!(readback, payload);
        let mut status = [0xff_u8];
        memory.read(GuestPhysAddr::new(STATUS), &mut status).unwrap();
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
            .write(
                GuestPhysAddr::new(invalid_used + 2),
                &0_u16.to_le_bytes(),
            )
            .unwrap();

        assert!(device.process_notified_queue_atomic(&mut memory).is_err());
        assert_eq!(device.sector0(), &original);
        let mut status = [0_u8];
        memory.read(GuestPhysAddr::new(STATUS), &mut status).unwrap();
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
            .write(
                GuestPhysAddr::new(DATA),
                &[0x5a; VIRTIO_BLK_SECTOR_SIZE],
            )
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
            .write(
                GuestPhysAddr::new(invalid_used + 2),
                &0_u16.to_le_bytes(),
            )
            .unwrap();

        assert!(device.process_notified_queue_atomic(&mut memory).is_err());
        let mut data = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
        memory.read(GuestPhysAddr::new(DATA), &mut data).unwrap();
        assert_eq!(data, [0x5a; VIRTIO_BLK_SECTOR_SIZE]);
        let mut status = [0_u8];
        memory.read(GuestPhysAddr::new(STATUS), &mut status).unwrap();
        assert_eq!(status, [0xff]);
        assert_eq!(device.last_avail_idx, 0);
        assert_eq!(device.last_used_idx, 0);
        assert!(device.notify_pending);
        assert_eq!(device.isr_status, 0);
    }
}
