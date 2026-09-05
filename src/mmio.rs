use crate::error::{Error, MmioError};
use crate::vcpu::{MmioDirection, MmioExit};

pub const BYTE_DEVICE_ADDRESS: u64 = 0x2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MmioService {
    Write,
    Read(Vec<u8>),
}

#[derive(Debug, Default)]
pub struct MmioBus {
    byte_device: Option<ByteMmioDevice>,
}

impl MmioBus {
    #[must_use]
    pub const fn empty() -> Self {
        Self { byte_device: None }
    }

    #[must_use]
    pub fn with_byte_device(read_value: u8) -> Self {
        Self {
            byte_device: Some(ByteMmioDevice {
                read_value,
                ..ByteMmioDevice::default()
            }),
        }
    }

    pub fn dispatch(&mut self, exit: &MmioExit) -> Result<MmioService, Error> {
        match self.byte_device.as_mut() {
            Some(device) if exit.address() == BYTE_DEVICE_ADDRESS => {
                device.handle(exit).map_err(Error::Mmio)
            }
            _ => Err(Error::Mmio(MmioError::UnhandledAddress {
                address: exit.address(),
                direction: exit.direction().raw(),
                length: exit.length(),
            })),
        }
    }

    #[must_use]
    pub fn writes(&self) -> Option<&[u8]> {
        self.byte_device.as_ref().map(ByteMmioDevice::writes)
    }
}

#[derive(Debug, Default)]
struct ByteMmioDevice {
    writes: Vec<u8>,
    read_value: u8,
}

impl ByteMmioDevice {
    fn handle(&mut self, exit: &MmioExit) -> Result<MmioService, MmioError> {
        if exit.length() != 1 {
            return Err(MmioError::UnsupportedByteDeviceAccess {
                address: exit.address(),
                direction: exit.direction().raw(),
                length: exit.length(),
            });
        }

        match exit.direction() {
            MmioDirection::Write => {
                if exit.write_data().len() != 1 {
                    return Err(MmioError::InvalidWritePayload {
                        address: exit.address(),
                        expected: 1,
                        actual: exit.write_data().len(),
                    });
                }
                self.writes.push(exit.write_data()[0]);
                Ok(MmioService::Write)
            }
            MmioDirection::Read => Ok(MmioService::Read(vec![self.read_value])),
        }
    }

    fn writes(&self) -> &[u8] {
        &self.writes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit(direction: MmioDirection, length: u32, write_data: &[u8]) -> MmioExit {
        MmioExit::new_for_test(BYTE_DEVICE_ADDRESS, direction, length, write_data.to_vec())
    }

    #[test]
    fn byte_device_captures_one_byte_write() {
        let mut bus = MmioBus::with_byte_device(b'R');
        assert_eq!(
            bus.dispatch(&exit(MmioDirection::Write, 1, b"W")).unwrap(),
            MmioService::Write
        );
        assert_eq!(bus.writes(), Some(&b"W"[..]));
    }

    #[test]
    fn byte_device_returns_configured_one_byte_read() {
        let mut bus = MmioBus::with_byte_device(b'R');
        assert_eq!(
            bus.dispatch(&exit(MmioDirection::Read, 1, &[])).unwrap(),
            MmioService::Read(vec![b'R'])
        );
    }

    #[test]
    fn rejects_unknown_address_wide_access_and_bad_write_payload() {
        let mut bus = MmioBus::with_byte_device(b'R');
        let unknown = MmioExit::new_for_test(0x3000, MmioDirection::Write, 1, b"X".to_vec());
        assert!(matches!(
            bus.dispatch(&unknown),
            Err(Error::Mmio(MmioError::UnhandledAddress {
                address: 0x3000,
                ..
            }))
        ));

        assert!(matches!(
            bus.dispatch(&exit(MmioDirection::Read, 2, &[])),
            Err(Error::Mmio(MmioError::UnsupportedByteDeviceAccess {
                length: 2,
                ..
            }))
        ));

        assert!(matches!(
            bus.dispatch(&exit(MmioDirection::Write, 1, b"AB")),
            Err(Error::Mmio(MmioError::InvalidWritePayload {
                expected: 1,
                actual: 2,
                ..
            }))
        ));
    }
}
