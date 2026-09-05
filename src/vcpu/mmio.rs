use super::{KvmRunMapping, Vcpu, VcpuId};
use crate::error::{Error, VmExitError};
use crate::kvm::sys;

pub(super) const KVM_EXIT_MMIO: u32 = 6;
pub const KVM_MMIO_DATA_CAPACITY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmioDirection {
    Read,
    Write,
}

impl MmioDirection {
    #[must_use]
    pub const fn raw(self) -> u8 {
        match self {
            Self::Read => 0,
            Self::Write => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MmioExit {
    address: u64,
    direction: MmioDirection,
    length: u32,
    write_data: Vec<u8>,
}

impl MmioExit {
    fn new(address: u64, direction: MmioDirection, length: u32, write_data: Vec<u8>) -> Self {
        Self {
            address,
            direction,
            length,
            write_data,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        address: u64,
        direction: MmioDirection,
        length: u32,
        write_data: Vec<u8>,
    ) -> Self {
        Self::new(address, direction, length, write_data)
    }

    #[must_use]
    pub const fn address(&self) -> u64 {
        self.address
    }

    #[must_use]
    pub const fn direction(&self) -> MmioDirection {
        self.direction
    }

    #[must_use]
    pub const fn length(&self) -> u32 {
        self.length
    }

    #[must_use]
    pub fn write_data(&self) -> &[u8] {
        &self.write_data
    }
}

impl Vcpu {
    pub(crate) fn mmio_exit(&self) -> Result<MmioExit, Error> {
        self.run.mmio_exit(self.id)
    }

    pub(crate) fn write_mmio_read_response(&mut self, response: &[u8]) -> Result<(), Error> {
        self.run.write_mmio_read_response(self.id, response)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KvmRunMmio {
    phys_addr: u64,
    data: [u8; KVM_MMIO_DATA_CAPACITY],
    len: u32,
    is_write: u8,
    padding: [u8; 3],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KvmRunMmioPrefix {
    header: sys::KvmRunHeader,
    cr8: u64,
    apic_base: u64,
    mmio: KvmRunMmio,
}

pub(super) const fn required_kvm_run_prefix_size() -> usize {
    std::mem::size_of::<KvmRunMmioPrefix>()
}

impl KvmRunMapping {
    fn mmio_exit(&self, id: VcpuId) -> Result<MmioExit, Error> {
        let exit_reason = self.exit_reason();
        if exit_reason != KVM_EXIT_MMIO {
            return Err(Error::VmExit(VmExitError::MmioPayloadUnavailable {
                vcpu_id: id.get(),
                exit_reason,
            }));
        }

        debug_assert!(self.len >= required_kvm_run_prefix_size());
        // SAFETY: `KvmRunMapping::map` requires enough bytes for every typed exit prefix used by
        // this crate. KVM places `struct kvm_run` at offset zero and mmap is suitably aligned.
        let prefix = unsafe { &*self.ptr.as_ptr().cast::<KvmRunMmioPrefix>() };
        decode_mmio(id, prefix.mmio)
    }

    fn write_mmio_read_response(&mut self, id: VcpuId, response: &[u8]) -> Result<(), Error> {
        let exit_reason = self.exit_reason();
        if exit_reason != KVM_EXIT_MMIO {
            return Err(Error::VmExit(VmExitError::MmioPayloadUnavailable {
                vcpu_id: id.get(),
                exit_reason,
            }));
        }

        debug_assert!(self.len >= required_kvm_run_prefix_size());
        // SAFETY: same layout proof as `mmio_exit`; this method has exclusive access to the
        // mapping while writing the userspace response into KVM's fixed MMIO data array.
        let prefix = unsafe { &mut *self.ptr.as_ptr().cast::<KvmRunMmioPrefix>() };
        let direction = mmio_direction(id, prefix.mmio.is_write)?;
        if direction != MmioDirection::Read {
            return Err(Error::VmExit(VmExitError::MmioResponseForWrite {
                vcpu_id: id.get(),
                address: prefix.mmio.phys_addr,
            }));
        }
        let length = checked_mmio_length(id, prefix.mmio.phys_addr, prefix.mmio.len)?;
        if response.len() != length {
            return Err(Error::VmExit(VmExitError::InvalidMmioResponseLength {
                vcpu_id: id.get(),
                address: prefix.mmio.phys_addr,
                expected: length,
                actual: response.len(),
            }));
        }
        prefix.mmio.data[..length].copy_from_slice(response);
        Ok(())
    }
}

fn decode_mmio(id: VcpuId, raw: KvmRunMmio) -> Result<MmioExit, Error> {
    let direction = mmio_direction(id, raw.is_write)?;
    let length = checked_mmio_length(id, raw.phys_addr, raw.len)?;
    let write_data = if direction == MmioDirection::Write {
        raw.data[..length].to_vec()
    } else {
        Vec::new()
    };
    Ok(MmioExit::new(
        raw.phys_addr,
        direction,
        raw.len,
        write_data,
    ))
}

fn mmio_direction(id: VcpuId, is_write: u8) -> Result<MmioDirection, Error> {
    match is_write {
        0 => Ok(MmioDirection::Read),
        1 => Ok(MmioDirection::Write),
        value => Err(Error::VmExit(VmExitError::InvalidMmioDirection {
            vcpu_id: id.get(),
            is_write: value,
        })),
    }
}

fn checked_mmio_length(id: VcpuId, address: u64, length: u32) -> Result<usize, Error> {
    let length_usize = usize::try_from(length).map_err(|_| {
        Error::VmExit(VmExitError::InvalidMmioLength {
            vcpu_id: id.get(),
            address,
            length,
            capacity: KVM_MMIO_DATA_CAPACITY,
        })
    })?;
    if length_usize == 0 || length_usize > KVM_MMIO_DATA_CAPACITY {
        return Err(Error::VmExit(VmExitError::InvalidMmioLength {
            vcpu_id: id.get(),
            address,
            length,
            capacity: KVM_MMIO_DATA_CAPACITY,
        }));
    }
    Ok(length_usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmio_prefix_matches_kvm_run_union_layout() {
        assert_eq!(KVM_EXIT_MMIO, 6);
        assert_eq!(std::mem::offset_of!(KvmRunMmioPrefix, mmio), 32);
        assert_eq!(std::mem::size_of::<KvmRunMmio>(), 24);
        assert_eq!(required_kvm_run_prefix_size(), 56);
    }

    #[test]
    fn decodes_mmio_write_into_owned_payload() {
        let mut raw = KvmRunMmio {
            phys_addr: 0x2000,
            len: 2,
            is_write: 1,
            ..KvmRunMmio::default()
        };
        raw.data[..2].copy_from_slice(&[0x34, 0x12]);
        let decoded = decode_mmio(VcpuId::BOOT, raw).unwrap();

        assert_eq!(decoded.address(), 0x2000);
        assert_eq!(decoded.direction(), MmioDirection::Write);
        assert_eq!(decoded.length(), 2);
        assert_eq!(decoded.write_data(), &[0x34, 0x12]);
    }

    #[test]
    fn decodes_mmio_read_without_exposing_stale_data() {
        let raw = KvmRunMmio {
            phys_addr: 0x2000,
            data: [0xaa; KVM_MMIO_DATA_CAPACITY],
            len: 1,
            is_write: 0,
            padding: [0; 3],
        };
        let decoded = decode_mmio(VcpuId::BOOT, raw).unwrap();

        assert_eq!(decoded.direction(), MmioDirection::Read);
        assert_eq!(decoded.length(), 1);
        assert!(decoded.write_data().is_empty());
    }

    #[test]
    fn rejects_zero_oversized_length_and_unknown_direction() {
        assert!(matches!(
            checked_mmio_length(VcpuId::new(2), 0x2000, 0),
            Err(Error::VmExit(VmExitError::InvalidMmioLength {
                vcpu_id: 2,
                length: 0,
                ..
            }))
        ));
        assert!(matches!(
            checked_mmio_length(VcpuId::BOOT, 0x2000, 9),
            Err(Error::VmExit(VmExitError::InvalidMmioLength { length: 9, .. }))
        ));
        assert!(matches!(
            mmio_direction(VcpuId::new(3), 2),
            Err(Error::VmExit(VmExitError::InvalidMmioDirection {
                vcpu_id: 3,
                is_write: 2,
            }))
        ));
    }
}
