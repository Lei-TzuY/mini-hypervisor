use super::*;

const DESCRIPTOR_SIZE: u32 = 16;
const MIN_REQUEST_DESCRIPTORS: u32 = 3;
const MAX_INDIRECT_DESCRIPTORS: u32 = u16::MAX as u32 + 1;

#[derive(Debug, Clone, Copy)]
pub(super) struct ResolvedRequestChain {
    pub(super) header: Descriptor,
    pub(super) data: Descriptor,
    pub(super) data_index: u16,
    pub(super) status: Descriptor,
}

impl VirtioBlkDevice {
    pub(super) fn resolve_request_chain(
        &self,
        memory: &GuestMemory,
        head: u16,
    ) -> Result<ResolvedRequestChain, VirtioBlkProcessError> {
        let outer = self.read_descriptor(memory, head)?;
        if outer.flags & VIRTQ_DESC_F_INDIRECT != 0 {
            self.resolve_indirect_request_chain(memory, head, outer)
        } else {
            self.resolve_direct_request_chain(memory, head, outer)
        }
    }

    fn resolve_direct_request_chain(
        &self,
        memory: &GuestMemory,
        head: u16,
        header: Descriptor,
    ) -> Result<ResolvedRequestChain, VirtioBlkProcessError> {
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

        Ok(ResolvedRequestChain {
            header,
            data,
            data_index,
            status,
        })
    }

    fn resolve_indirect_request_chain(
        &self,
        memory: &GuestMemory,
        head: u16,
        outer: Descriptor,
    ) -> Result<ResolvedRequestChain, VirtioBlkProcessError> {
        if self.driver_features & VIRTIO_RING_F_INDIRECT_DESC == 0 {
            return Err(VirtioBlkError::IndirectFeatureNotNegotiated.into());
        }
        self.require_flags(head, outer.flags, VIRTQ_DESC_F_INDIRECT)?;
        if outer.length < MIN_REQUEST_DESCRIPTORS * DESCRIPTOR_SIZE
            || outer.length % DESCRIPTOR_SIZE != 0
        {
            return Err(VirtioBlkError::InvalidIndirectTableLength {
                length: outer.length,
                descriptor_size: DESCRIPTOR_SIZE,
            }
            .into());
        }
        let entries = outer.length / DESCRIPTOR_SIZE;
        if entries > MAX_INDIRECT_DESCRIPTORS {
            return Err(VirtioBlkError::IndirectTableTooLarge {
                entries,
                maximum: MAX_INDIRECT_DESCRIPTORS,
            }
            .into());
        }

        let header = self.read_indirect_descriptor(memory, outer.address, entries, 0)?;
        self.reject_nested_indirect(0, header.flags)?;
        self.require_flags(0, header.flags, VIRTQ_DESC_F_NEXT)?;
        self.require_length(0, header.length, 16)?;

        let data_index = header.next;
        if data_index == 0 {
            return Err(VirtioBlkError::DescriptorChainCycle { index: data_index }.into());
        }
        let data = self.read_indirect_descriptor(memory, outer.address, entries, data_index)?;
        self.reject_nested_indirect(data_index, data.flags)?;

        let status_index = data.next;
        if status_index == 0 || status_index == data_index {
            return Err(VirtioBlkError::DescriptorChainCycle {
                index: status_index,
            }
            .into());
        }
        let status = self.read_indirect_descriptor(memory, outer.address, entries, status_index)?;
        self.reject_nested_indirect(status_index, status.flags)?;
        self.require_flags(status_index, status.flags, VIRTQ_DESC_F_WRITE)?;
        self.require_length(status_index, status.length, 1)?;

        Ok(ResolvedRequestChain {
            header,
            data,
            data_index,
            status,
        })
    }

    fn read_indirect_descriptor(
        &self,
        memory: &GuestMemory,
        table: u64,
        entries: u32,
        index: u16,
    ) -> Result<Descriptor, VirtioBlkProcessError> {
        if u32::from(index) >= entries {
            return Err(
                VirtioBlkError::IndirectDescriptorIndexOutOfRange { index, entries }.into(),
            );
        }
        let address = checked_add(table, DESCRIPTOR_SIZE as u64 * u64::from(index))?;
        let mut bytes = [0_u8; DESCRIPTOR_SIZE as usize];
        memory.read(GuestPhysAddr::new(address), &mut bytes)?;
        Ok(Descriptor {
            address: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            length: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            flags: u16::from_le_bytes(bytes[12..14].try_into().unwrap()),
            next: u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
        })
    }

    fn reject_nested_indirect(&self, index: u16, flags: u16) -> Result<(), VirtioBlkError> {
        if flags & VIRTQ_DESC_F_INDIRECT != 0 {
            Err(VirtioBlkError::NestedIndirectDescriptor { index })
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: u64 = 0x1000_0000;
    const DESC: u64 = 0x18000;
    const AVAIL: u64 = 0x18100;
    const USED: u64 = 0x18200;
    const TABLE: u64 = 0x18300;
    const HEADER: u64 = 0x18400;
    const DATA: u64 = 0x18500;
    const STATUS: u64 = 0x18800;
    const MEMORY_SIZE: u64 = 0x20_000;

    fn ready_device() -> VirtioBlkDevice {
        let mut device = VirtioBlkDevice::new(BAR);
        device.driver_features = VIRTIO_F_VERSION_1 | VIRTIO_RING_F_INDIRECT_DESC;
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
        table: u64,
        index: u16,
        address: u64,
        length: u32,
        flags: u16,
        next: u16,
    ) {
        let mut bytes = [0_u8; 16];
        bytes[0..8].copy_from_slice(&address.to_le_bytes());
        bytes[8..12].copy_from_slice(&length.to_le_bytes());
        bytes[12..14].copy_from_slice(&flags.to_le_bytes());
        bytes[14..16].copy_from_slice(&next.to_le_bytes());
        memory
            .write(GuestPhysAddr::new(table + 16 * u64::from(index)), &bytes)
            .unwrap();
    }

    fn prepare_indirect_request(
        memory: &mut GuestMemory,
        device: &mut VirtioBlkDevice,
        request_type: u32,
        data_flags: u16,
        avail_idx: u16,
        ring_slot: u16,
    ) {
        write_descriptor(memory, DESC, 0, TABLE, 48, VIRTQ_DESC_F_INDIRECT, 0);
        write_descriptor(memory, TABLE, 0, HEADER, 16, VIRTQ_DESC_F_NEXT, 1);
        write_descriptor(
            memory,
            TABLE,
            1,
            DATA,
            VIRTIO_BLK_SECTOR_SIZE as u32,
            data_flags,
            2,
        );
        write_descriptor(memory, TABLE, 2, STATUS, 1, VIRTQ_DESC_F_WRITE, 0);
        let mut header = [0_u8; 16];
        header[0..4].copy_from_slice(&request_type.to_le_bytes());
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

    #[test]
    fn indirect_out_then_in_uses_outer_head_and_round_trips_backing() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MEMORY_SIZE).unwrap();
        let mut device = ready_device();
        let mut payload = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(13).wrapping_add(9);
        }
        memory.write(GuestPhysAddr::new(DATA), &payload).unwrap();
        prepare_indirect_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_OUT,
            VIRTQ_DESC_F_NEXT,
            1,
            0,
        );

        let write = device.process_notified_queue_atomic(&mut memory).unwrap();
        assert_eq!(write.descriptor_id(), 0);
        assert_eq!(write.length(), 1);
        assert_eq!(device.sector0(), &payload);

        memory
            .write(GuestPhysAddr::new(DATA), &[0x5a; VIRTIO_BLK_SECTOR_SIZE])
            .unwrap();
        prepare_indirect_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_IN,
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
            2,
            1,
        );
        let read = device.process_notified_queue_atomic(&mut memory).unwrap();
        assert_eq!(read.descriptor_id(), 0);
        assert_eq!(read.length(), (VIRTIO_BLK_SECTOR_SIZE + 1) as u32);
        let mut readback = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
        memory
            .read(GuestPhysAddr::new(DATA), &mut readback)
            .unwrap();
        assert_eq!(readback, payload);
        assert_eq!(read_guest_u16(&memory, USED + 2).unwrap(), 2);
    }

    #[test]
    fn indirect_head_requires_negotiated_feature_without_mutation() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MEMORY_SIZE).unwrap();
        let mut device = ready_device();
        device.driver_features = VIRTIO_F_VERSION_1;
        let original = *device.sector0();
        prepare_indirect_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_OUT,
            VIRTQ_DESC_F_NEXT,
            1,
            0,
        );

        let error = device
            .process_notified_queue_atomic(&mut memory)
            .unwrap_err();
        assert!(matches!(
            error,
            VirtioBlkProcessError::Device(VirtioBlkError::IndirectFeatureNotNegotiated)
        ));
        assert_eq!(device.sector0(), &original);
        assert_eq!(device.last_avail_idx, 0);
        assert_eq!(device.last_used_idx, 0);
        assert!(device.notify_pending);
        assert_eq!(device.isr_status, 0);
    }

    #[test]
    fn invalid_indirect_topology_fails_before_queue_or_backing_mutation() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), MEMORY_SIZE).unwrap();
        let mut device = ready_device();
        let original = *device.sector0();
        prepare_indirect_request(
            &mut memory,
            &mut device,
            VIRTIO_BLK_T_OUT,
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_INDIRECT,
            1,
            0,
        );

        let error = device
            .process_notified_queue_atomic(&mut memory)
            .unwrap_err();
        assert!(matches!(
            error,
            VirtioBlkProcessError::Device(VirtioBlkError::NestedIndirectDescriptor { index: 1 })
        ));
        assert_eq!(device.sector0(), &original);
        assert_eq!(device.last_avail_idx, 0);
        assert_eq!(device.last_used_idx, 0);
        assert!(device.notify_pending);
        assert_eq!(device.isr_status, 0);
    }
}
