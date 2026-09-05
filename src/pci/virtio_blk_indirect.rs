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
    pub(super) status_index: u16,
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
            status_index,
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
            status_index,
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
            return Err(VirtioBlkError::IndirectDescriptorIndexOutOfRange { index, entries }.into());
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
