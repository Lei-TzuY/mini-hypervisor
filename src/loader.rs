use crate::error::{Error, GuestImageError};
use crate::memory::{GuestMemory, GuestPhysAddr};

#[derive(Debug, Clone, Copy)]
pub struct FlatGuestImage<'a> {
    load_address: GuestPhysAddr,
    entry: GuestPhysAddr,
    bytes: &'a [u8],
}

impl<'a> FlatGuestImage<'a> {
    pub fn new(
        load_address: GuestPhysAddr,
        entry: GuestPhysAddr,
        bytes: &'a [u8],
    ) -> Result<Self, Error> {
        if bytes.is_empty() {
            return Err(Error::GuestImage(GuestImageError::EmptyFlatBinary));
        }

        let length_u64 = u64::try_from(bytes.len()).map_err(|_| {
            Error::GuestImage(GuestImageError::ImageLengthTooLarge {
                length: bytes.len(),
            })
        })?;
        let image_end = load_address
            .get()
            .checked_add(length_u64)
            .ok_or_else(|| {
                Error::GuestImage(GuestImageError::ImageRangeOverflow {
                    load_address: load_address.get(),
                    length: bytes.len(),
                })
            })?;

        if entry.get() < load_address.get() || entry.get() >= image_end {
            return Err(Error::GuestImage(GuestImageError::EntryOutsideImage {
                entry: entry.get(),
                load_address: load_address.get(),
                length: bytes.len(),
            }));
        }

        Ok(Self {
            load_address,
            entry,
            bytes,
        })
    }

    #[must_use]
    pub const fn load_address(&self) -> GuestPhysAddr {
        self.load_address
    }

    #[must_use]
    pub const fn entry(&self) -> GuestPhysAddr {
        self.entry
    }

    #[must_use]
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn load(&self, memory: &mut GuestMemory) -> Result<(), Error> {
        memory.write(self.load_address, self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::GuestMemoryError;
    use crate::memory::KVM_MEMORY_ALIGNMENT;

    const LOAD_ADDRESS: GuestPhysAddr = GuestPhysAddr::new(0x1000);

    #[test]
    fn accepts_entry_at_image_boundaries() {
        let bytes = [0x90, 0xf4];
        assert!(FlatGuestImage::new(LOAD_ADDRESS, LOAD_ADDRESS, &bytes).is_ok());
        assert!(FlatGuestImage::new(
            LOAD_ADDRESS,
            GuestPhysAddr::new(LOAD_ADDRESS.get() + 1),
            &bytes
        )
        .is_ok());
    }

    #[test]
    fn rejects_empty_image() {
        assert!(matches!(
            FlatGuestImage::new(LOAD_ADDRESS, LOAD_ADDRESS, &[]),
            Err(Error::GuestImage(GuestImageError::EmptyFlatBinary))
        ));
    }

    #[test]
    fn rejects_entry_at_exclusive_end() {
        let bytes = [0xf4];
        assert!(matches!(
            FlatGuestImage::new(
                LOAD_ADDRESS,
                GuestPhysAddr::new(LOAD_ADDRESS.get() + 1),
                &bytes
            ),
            Err(Error::GuestImage(GuestImageError::EntryOutsideImage { .. }))
        ));
    }

    #[test]
    fn rejects_overflowed_image_range() {
        let bytes = [0x90, 0xf4];
        let load_address = GuestPhysAddr::new(u64::MAX);
        assert!(matches!(
            FlatGuestImage::new(load_address, load_address, &bytes),
            Err(Error::GuestImage(GuestImageError::ImageRangeOverflow { .. }))
        ));
    }

    #[test]
    fn loads_exact_bytes_into_guest_memory() {
        let bytes = [0x90, 0xf4];
        let image = FlatGuestImage::new(LOAD_ADDRESS, LOAD_ADDRESS, &bytes).unwrap();
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), KVM_MEMORY_ALIGNMENT * 2).unwrap();
        image.load(&mut memory).unwrap();

        let mut actual = [0; 2];
        memory.read(LOAD_ADDRESS, &mut actual).unwrap();
        assert_eq!(actual, bytes);
    }

    #[test]
    fn loading_image_cannot_cross_guest_ram_end() {
        let bytes = [0x90, 0xf4];
        let load_address = GuestPhysAddr::new(KVM_MEMORY_ALIGNMENT - 1);
        let image = FlatGuestImage::new(load_address, load_address, &bytes).unwrap();
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), KVM_MEMORY_ALIGNMENT).unwrap();

        assert!(matches!(
            image.load(&mut memory),
            Err(Error::GuestMemory(
                GuestMemoryError::AccessOutOfBounds { .. }
            ))
        ));
    }
}
