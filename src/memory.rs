use crate::error::{Error, GuestMemoryError};
use std::io;
use std::ptr::NonNull;

pub const KVM_MEMORY_ALIGNMENT: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct GuestPhysAddr(u64);

impl GuestPhysAddr {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestMemoryRegion {
    base: GuestPhysAddr,
    size: u64,
}

impl GuestMemoryRegion {
    pub fn new(base: GuestPhysAddr, size: u64) -> Result<Self, Error> {
        if size == 0 {
            return Err(Error::GuestMemory(GuestMemoryError::ZeroSizedRegion));
        }
        if base.get() % KVM_MEMORY_ALIGNMENT != 0 {
            return Err(Error::GuestMemory(GuestMemoryError::MisalignedRegion {
                field: "guest physical base",
                value: base.get(),
                alignment: KVM_MEMORY_ALIGNMENT,
            }));
        }
        if size % KVM_MEMORY_ALIGNMENT != 0 {
            return Err(Error::GuestMemory(GuestMemoryError::MisalignedRegion {
                field: "memory size",
                value: size,
                alignment: KVM_MEMORY_ALIGNMENT,
            }));
        }
        base.get().checked_add(size).ok_or_else(|| {
            Error::GuestMemory(GuestMemoryError::AddressSpaceOverflow {
                base: base.get(),
                size,
            })
        })?;

        Ok(Self { base, size })
    }

    #[must_use]
    pub const fn base(self) -> GuestPhysAddr {
        self.base
    }

    #[must_use]
    pub const fn size(self) -> u64 {
        self.size
    }

    #[must_use]
    pub fn end(self) -> GuestPhysAddr {
        GuestPhysAddr::new(
            self.base
                .get()
                .checked_add(self.size)
                .expect("region construction validates the exclusive end"),
        )
    }

    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.base.get() < other.end().get() && other.base.get() < self.end().get()
    }

    fn checked_offset(self, address: GuestPhysAddr, length: usize) -> Result<usize, Error> {
        let length_u64 = u64::try_from(length)
            .map_err(|_| Error::GuestMemory(GuestMemoryError::AccessLengthTooLarge { length }))?;
        let access_end = address.get().checked_add(length_u64).ok_or_else(|| {
            Error::GuestMemory(GuestMemoryError::AccessOverflow {
                address: address.get(),
                length,
            })
        })?;
        let region_end = self.end().get();

        if address.get() < self.base.get()
            || access_end > region_end
            || (length != 0 && address.get() >= region_end)
        {
            return Err(Error::GuestMemory(GuestMemoryError::AccessOutOfBounds {
                address: address.get(),
                length,
                region_base: self.base.get(),
                region_size: self.size,
            }));
        }

        let offset = address
            .get()
            .checked_sub(self.base.get())
            .expect("lower bound was checked above");
        usize::try_from(offset)
            .map_err(|_| Error::GuestMemory(GuestMemoryError::HostSizeOverflow { size: offset }))
    }
}

#[derive(Debug)]
pub struct GuestMemory {
    region: GuestMemoryRegion,
    mapping: RamMapping,
}

impl GuestMemory {
    pub fn new(base: GuestPhysAddr, size: u64) -> Result<Self, Error> {
        let region = GuestMemoryRegion::new(base, size)?;
        let host_size = usize::try_from(size)
            .map_err(|_| Error::GuestMemory(GuestMemoryError::HostSizeOverflow { size }))?;
        let mapping = RamMapping::new(host_size)
            .map_err(|source| Error::GuestMemory(GuestMemoryError::Mapping { source }))?;

        Ok(Self { region, mapping })
    }

    #[must_use]
    pub const fn region(&self) -> GuestMemoryRegion {
        self.region
    }

    pub fn read(&self, address: GuestPhysAddr, destination: &mut [u8]) -> Result<(), Error> {
        let offset = self.region.checked_offset(address, destination.len())?;
        if destination.is_empty() {
            return Ok(());
        }

        // SAFETY: `checked_offset` proves the entire destination length is inside the mmap.
        let source = unsafe { self.mapping.ptr.as_ptr().add(offset) };
        // SAFETY: source points to at least destination.len() readable mapped bytes and the
        // destination slice is valid and non-overlapping with this private mmap.
        unsafe {
            std::ptr::copy_nonoverlapping(source, destination.as_mut_ptr(), destination.len());
        }
        Ok(())
    }

    pub fn write(&mut self, address: GuestPhysAddr, source: &[u8]) -> Result<(), Error> {
        let offset = self.region.checked_offset(address, source.len())?;
        if source.is_empty() {
            return Ok(());
        }

        // SAFETY: `checked_offset` proves the entire source length fits inside the mmap.
        let destination = unsafe { self.mapping.ptr.as_ptr().add(offset) };
        // SAFETY: destination points to at least source.len() writable mapped bytes and the
        // source slice is valid and non-overlapping with this private mmap.
        unsafe {
            std::ptr::copy_nonoverlapping(source.as_ptr(), destination, source.len());
        }
        Ok(())
    }

    pub(crate) fn userspace_addr(&self) -> u64 {
        let address = self.mapping.ptr.as_ptr() as usize;
        u64::try_from(address).expect("x86-64 host virtual addresses fit in u64")
    }
}

#[derive(Debug)]
struct RamMapping {
    ptr: NonNull<u8>,
    len: usize,
}

impl RamMapping {
    fn new(len: usize) -> io::Result<Self> {
        // SAFETY: arguments request a new private anonymous mapping and do not alias an existing
        // Rust allocation. A successful mapping is owned exclusively by `RamMapping`.
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        let Some(ptr) = NonNull::new(raw.cast::<u8>()) else {
            // SAFETY: `raw` is a successful mapping of exactly `len` bytes.
            let _ = unsafe { libc::munmap(raw, len) };
            return Err(io::Error::other(
                "mmap unexpectedly returned a null address",
            ));
        };
        Ok(Self { ptr, len })
    }
}

impl Drop for RamMapping {
    fn drop(&mut self) {
        // SAFETY: this object owns the mapping at `ptr` for exactly `len` bytes and drops once.
        let _ = unsafe { libc::munmap(self.ptr.as_ptr().cast(), self.len) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: GuestPhysAddr = GuestPhysAddr::new(0x20_0000);
    const SIZE: u64 = 2 * KVM_MEMORY_ALIGNMENT;

    fn region() -> GuestMemoryRegion {
        GuestMemoryRegion::new(BASE, SIZE).unwrap()
    }

    #[test]
    fn accepts_first_and_last_byte() {
        let region = region();
        assert_eq!(region.checked_offset(BASE, 1).unwrap(), 0);
        assert_eq!(
            region
                .checked_offset(GuestPhysAddr::new(BASE.get() + SIZE - 1), 1)
                .unwrap(),
            usize::try_from(SIZE - 1).unwrap()
        );
    }

    #[test]
    fn accepts_zero_length_at_exact_end() {
        assert_eq!(
            region()
                .checked_offset(GuestPhysAddr::new(BASE.get() + SIZE), 0)
                .unwrap(),
            usize::try_from(SIZE).unwrap()
        );
    }

    #[test]
    fn rejects_nonzero_access_at_exact_end() {
        assert!(matches!(
            region().checked_offset(GuestPhysAddr::new(BASE.get() + SIZE), 1),
            Err(Error::GuestMemory(
                GuestMemoryError::AccessOutOfBounds { .. }
            ))
        ));
    }

    #[test]
    fn rejects_access_before_region() {
        assert!(matches!(
            region().checked_offset(GuestPhysAddr::new(BASE.get() - 1), 1),
            Err(Error::GuestMemory(
                GuestMemoryError::AccessOutOfBounds { .. }
            ))
        ));
    }

    #[test]
    fn rejects_access_crossing_region_end() {
        assert!(matches!(
            region().checked_offset(GuestPhysAddr::new(BASE.get() + SIZE - 1), 2),
            Err(Error::GuestMemory(
                GuestMemoryError::AccessOutOfBounds { .. }
            ))
        ));
    }

    #[test]
    fn rejects_overflowed_access_end() {
        let base = GuestPhysAddr::new(u64::MAX - (2 * KVM_MEMORY_ALIGNMENT - 1));
        let region = GuestMemoryRegion::new(base, KVM_MEMORY_ALIGNMENT).unwrap();
        assert!(matches!(
            region.checked_offset(GuestPhysAddr::new(u64::MAX - 1), 4),
            Err(Error::GuestMemory(GuestMemoryError::AccessOverflow { .. }))
        ));
    }

    #[test]
    fn rejects_overflowed_region_end() {
        let base = GuestPhysAddr::new(u64::MAX - (KVM_MEMORY_ALIGNMENT - 1));
        assert!(matches!(
            GuestMemoryRegion::new(base, KVM_MEMORY_ALIGNMENT),
            Err(Error::GuestMemory(
                GuestMemoryError::AddressSpaceOverflow { .. }
            ))
        ));
    }

    #[test]
    fn rejects_zero_sized_region() {
        assert!(matches!(
            GuestMemoryRegion::new(BASE, 0),
            Err(Error::GuestMemory(GuestMemoryError::ZeroSizedRegion))
        ));
    }

    #[test]
    fn rejects_misaligned_base_and_size() {
        assert!(matches!(
            GuestMemoryRegion::new(GuestPhysAddr::new(1), KVM_MEMORY_ALIGNMENT),
            Err(Error::GuestMemory(
                GuestMemoryError::MisalignedRegion { .. }
            ))
        ));
        assert!(matches!(
            GuestMemoryRegion::new(BASE, KVM_MEMORY_ALIGNMENT + 1),
            Err(Error::GuestMemory(
                GuestMemoryError::MisalignedRegion { .. }
            ))
        ));
    }

    #[test]
    fn detects_overlap_and_adjacency() {
        let first =
            GuestMemoryRegion::new(GuestPhysAddr::new(0), 2 * KVM_MEMORY_ALIGNMENT).unwrap();
        let overlapping = GuestMemoryRegion::new(
            GuestPhysAddr::new(KVM_MEMORY_ALIGNMENT),
            KVM_MEMORY_ALIGNMENT,
        )
        .unwrap();
        let adjacent = GuestMemoryRegion::new(
            GuestPhysAddr::new(2 * KVM_MEMORY_ALIGNMENT),
            KVM_MEMORY_ALIGNMENT,
        )
        .unwrap();
        assert!(first.overlaps(overlapping));
        assert!(!first.overlaps(adjacent));
    }

    #[test]
    fn mapped_memory_round_trips_boundary_bytes() {
        let mut memory = GuestMemory::new(BASE, KVM_MEMORY_ALIGNMENT).unwrap();
        memory.write(BASE, &[0x5a]).unwrap();
        memory
            .write(
                GuestPhysAddr::new(BASE.get() + KVM_MEMORY_ALIGNMENT - 1),
                &[0xa5],
            )
            .unwrap();

        let mut first = [0];
        let mut last = [0];
        memory.read(BASE, &mut first).unwrap();
        memory
            .read(
                GuestPhysAddr::new(BASE.get() + KVM_MEMORY_ALIGNMENT - 1),
                &mut last,
            )
            .unwrap();
        assert_eq!(first, [0x5a]);
        assert_eq!(last, [0xa5]);
    }
}
