use super::{KvmRunMapping, Vcpu, VcpuId};
use crate::error::{Error, VmExitError};
use crate::kvm::sys;

pub(super) const KVM_EXIT_MMIO: u32 = 6;
const KVM_MMIO_DATA_CAPACITY: usize = 8;
const KVM_MMIO_READ: u8 = 0;
const KVM_MMIO_WRITE: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuMmioDirection {
    Read,
    Write,
}

impl VcpuMmioDirection {
    #[must_use]
    pub const fn is_write(self) -> bool {
        matches!(self, Self::Write)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcpuMmioExit {
    phys_addr: u64,
    direction: VcpuMmioDirection,
    length: u32,
    write_data: Vec<u8>,
}

impl VcpuMmioExit {
    #[must_use]
    pub const fn phys_addr(&self) -> u64 {
        self.phys_addr
    }

    #[must_use]
    pub const fn direction(&self) -> VcpuMmioDirection {
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
    pub fn mmio_exit(&self) -> Result<VcpuMmioExit, Error> {
        self.run.mmio_exit(self.id)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmRunMmio {
    phys_addr: u64,
    data: [u8; KVM_MMIO_DATA_CAPACITY],
    len: u32,
    is_write: u8,
    padding: [u8; 3],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    fn mmio_exit(&self, id: VcpuId) -> Result<VcpuMmioExit, Error> {
        let exit_reason = self.exit_reason();
        if exit_reason != KVM_EXIT_MMIO {
            return Err(Error::VmExit(VmExitError::MmioPayloadUnavailable {
                vcpu_id: id.get(),
                exit_reason,
            }));
        }

        debug_assert!(self.len >= required_kvm_run_prefix_size());
        // SAFETY: `KvmRunMapping::map` rejects mappings smaller than every typed prefix used by
        // this crate, KVM places `struct kvm_run` at offset zero, and mmap returns suitably
        // aligned memory.
        let prefix = unsafe { &*self.ptr.as_ptr().cast::<KvmRunMmioPrefix>() };
        decode_mmio(id, &prefix.mmio)
    }
}

fn decode_mmio(id: VcpuId, raw: &KvmRunMmio) -> Result<VcpuMmioExit, Error> {
    let direction = match raw.is_write {
        KVM_MMIO_READ => VcpuMmioDirection::Read,
        KVM_MMIO_WRITE => VcpuMmioDirection::Write,
        is_write => {
            return Err(Error::VmExit(VmExitError::InvalidMmioDirection {
                vcpu_id: id.get(),
                is_write,
                exit_reasons: vec![KVM_EXIT_MMIO],
            }));
        }
    };

    let length = usize::try_from(raw.len).expect("u32 MMIO length fits usize");
    if length > KVM_MMIO_DATA_CAPACITY {
        return Err(Error::VmExit(VmExitError::InvalidMmioLength {
            vcpu_id: id.get(),
            length: raw.len,
            capacity: KVM_MMIO_DATA_CAPACITY,
            exit_reasons: vec![KVM_EXIT_MMIO],
        }));
    }

    // KVM documents `data` as userspace output for MMIO reads. Do not inspect it until a future
    // read-response path exists; only write exits carry kernel-provided bytes for userspace.
    let write_data = if direction == VcpuMmioDirection::Write {
        raw.data[..length].to_vec()
    } else {
        Vec::new()
    };

    Ok(VcpuMmioExit {
        phys_addr: raw.phys_addr,
        direction,
        length: raw.len,
        write_data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_mmio(phys_addr: u64, len: u32, is_write: u8, data: [u8; 8]) -> KvmRunMmio {
        KvmRunMmio {
            phys_addr,
            data,
            len,
            is_write,
            padding: [0; 3],
        }
    }

    #[test]
    fn mmio_prefix_matches_kvm_run_union_layout() {
        assert_eq!(std::mem::offset_of!(KvmRunMmioPrefix, mmio), 32);
        assert_eq!(std::mem::size_of::<KvmRunMmio>(), 24);
        assert_eq!(required_kvm_run_prefix_size(), 56);
    }

    #[test]
    fn write_decoder_copies_only_declared_bytes() {
        let decoded = decode_mmio(
            VcpuId::new(4),
            &raw_mmio(0xfee0_0010, 3, KVM_MMIO_WRITE, [1, 2, 3, 4, 5, 6, 7, 8]),
        )
        .unwrap();

        assert_eq!(decoded.phys_addr(), 0xfee0_0010);
        assert_eq!(decoded.direction(), VcpuMmioDirection::Write);
        assert_eq!(decoded.length(), 3);
        assert_eq!(decoded.write_data(), [1, 2, 3]);
    }

    #[test]
    fn read_decoder_does_not_publish_pending_response_buffer() {
        let decoded = decode_mmio(
            VcpuId::BOOT,
            &raw_mmio(0xf000_1000, 4, KVM_MMIO_READ, [0xaa; 8]),
        )
        .unwrap();

        assert_eq!(decoded.direction(), VcpuMmioDirection::Read);
        assert_eq!(decoded.length(), 4);
        assert!(decoded.write_data().is_empty());
    }

    #[test]
    fn accepts_full_mmio_write_capacity() {
        let decoded = decode_mmio(
            VcpuId::BOOT,
            &raw_mmio(0x1000, 8, KVM_MMIO_WRITE, [0, 1, 2, 3, 4, 5, 6, 7]),
        )
        .unwrap();

        assert_eq!(decoded.write_data(), [0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn rejects_mmio_length_above_fixed_kvm_capacity() {
        let error = decode_mmio(
            VcpuId::new(9),
            &raw_mmio(0x2000, 9, KVM_MMIO_WRITE, [0; 8]),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            Error::VmExit(VmExitError::InvalidMmioLength {
                vcpu_id: 9,
                length: 9,
                capacity: 8,
                exit_reasons,
            }) if exit_reasons == [KVM_EXIT_MMIO]
        ));
    }

    #[test]
    fn rejects_unknown_mmio_write_flag() {
        let error = decode_mmio(VcpuId::new(7), &raw_mmio(0x3000, 4, 2, [0; 8])).unwrap_err();

        assert!(matches!(
            error,
            Error::VmExit(VmExitError::InvalidMmioDirection {
                vcpu_id: 7,
                is_write: 2,
                exit_reasons,
            }) if exit_reasons == [KVM_EXIT_MMIO]
        ));
    }
}
