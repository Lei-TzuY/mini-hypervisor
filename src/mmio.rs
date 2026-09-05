use crate::error::{Error, MmioError};
use crate::vcpu::{MmioDirection, MmioExit};

pub mod interrupt;
pub mod long_mode;

pub const BYTE_DEVICE_ADDRESS: u64 = 0x2000;
pub const LEVEL_INTERRUPT_STATUS_OFFSET: u64 = 1;
pub const LEVEL_INTERRUPT_ACK_OFFSET: u64 = 2;
pub const LEVEL_INTERRUPT_STATUS_PENDING: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MmioService {
    Write,
    Read(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmioDeviceEvent {
    InterruptRequested,
    InterruptLineAsserted,
    InterruptLineDeasserted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByteMmioDeviceMode {
    Plain,
    EdgeInterrupt,
    LevelInterrupt,
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
        Self::with_byte_device_at(BYTE_DEVICE_ADDRESS, read_value)
    }

    #[must_use]
    pub fn with_byte_device_at(address: u64, read_value: u8) -> Self {
        Self {
            byte_device: Some(ByteMmioDevice::new(
                address,
                read_value,
                ByteMmioDeviceMode::Plain,
            )),
        }
    }

    #[must_use]
    pub fn with_interrupting_byte_device_at(address: u64, read_value: u8) -> Self {
        Self {
            byte_device: Some(ByteMmioDevice::new(
                address,
                read_value,
                ByteMmioDeviceMode::EdgeInterrupt,
            )),
        }
    }

    #[must_use]
    pub fn with_level_interrupt_byte_device_at(address: u64) -> Self {
        Self {
            byte_device: Some(ByteMmioDevice::new(
                address,
                0,
                ByteMmioDeviceMode::LevelInterrupt,
            )),
        }
    }

    pub fn dispatch(&mut self, exit: &MmioExit) -> Result<MmioService, Error> {
        match self.byte_device.as_mut() {
            Some(device) if device.handles(exit.address()) => device.handle(exit).map_err(Error::Mmio),
            _ => Err(Error::Mmio(MmioError::UnhandledAddress {
                address: exit.address(),
                direction: exit.direction().raw(),
                length: exit.length(),
            })),
        }
    }

    pub fn take_device_event(&mut self) -> Option<MmioDeviceEvent> {
        self.byte_device
            .as_mut()
            .and_then(ByteMmioDevice::take_event)
    }

    #[must_use]
    pub fn writes(&self) -> Option<&[u8]> {
        self.byte_device.as_ref().map(ByteMmioDevice::writes)
    }
}

#[derive(Debug)]
struct ByteMmioDevice {
    address: u64,
    writes: Vec<u8>,
    read_value: u8,
    mode: ByteMmioDeviceMode,
    edge_interrupt_pending: bool,
    level_interrupt_pending: bool,
    level_line_asserted: bool,
}

impl ByteMmioDevice {
    fn new(address: u64, read_value: u8, mode: ByteMmioDeviceMode) -> Self {
        Self {
            address,
            writes: Vec::new(),
            read_value,
            mode,
            edge_interrupt_pending: false,
            level_interrupt_pending: false,
            level_line_asserted: false,
        }
    }

    fn handles(&self, address: u64) -> bool {
        match self.mode {
            ByteMmioDeviceMode::Plain | ByteMmioDeviceMode::EdgeInterrupt => {
                address == self.address
            }
            ByteMmioDeviceMode::LevelInterrupt => {
                self.address
                    .checked_add(LEVEL_INTERRUPT_ACK_OFFSET)
                    .is_some_and(|end| (self.address..=end).contains(&address))
            }
        }
    }

    fn handle(&mut self, exit: &MmioExit) -> Result<MmioService, MmioError> {
        if exit.length() != 1 {
            return Err(MmioError::UnsupportedByteDeviceAccess {
                address: exit.address(),
                direction: exit.direction().raw(),
                length: exit.length(),
            });
        }

        match self.mode {
            ByteMmioDeviceMode::Plain => self.handle_plain(exit, false),
            ByteMmioDeviceMode::EdgeInterrupt => self.handle_plain(exit, true),
            ByteMmioDeviceMode::LevelInterrupt => self.handle_level(exit),
        }
    }

    fn handle_plain(
        &mut self,
        exit: &MmioExit,
        interrupt_on_write: bool,
    ) -> Result<MmioService, MmioError> {
        match exit.direction() {
            MmioDirection::Write => {
                let value = exact_write_byte(exit)?;
                self.writes.push(value);
                if interrupt_on_write {
                    self.edge_interrupt_pending = true;
                }
                Ok(MmioService::Write)
            }
            MmioDirection::Read => Ok(MmioService::Read(vec![self.read_value])),
        }
    }

    fn handle_level(&mut self, exit: &MmioExit) -> Result<MmioService, MmioError> {
        let offset = exit.address() - self.address;
        match (offset, exit.direction()) {
            (0, MmioDirection::Write) => {
                let value = exact_write_byte(exit)?;
                self.writes.push(value);
                self.level_interrupt_pending = true;
                Ok(MmioService::Write)
            }
            (LEVEL_INTERRUPT_STATUS_OFFSET, MmioDirection::Read) => Ok(MmioService::Read(vec![
                u8::from(self.level_interrupt_pending),
            ])),
            (LEVEL_INTERRUPT_ACK_OFFSET, MmioDirection::Write) => {
                let value = exact_write_byte(exit)?;
                self.writes.push(value);
                self.level_interrupt_pending = false;
                Ok(MmioService::Write)
            }
            _ => Err(MmioError::UnsupportedByteDeviceAccess {
                address: exit.address(),
                direction: exit.direction().raw(),
                length: exit.length(),
            }),
        }
    }

    fn take_event(&mut self) -> Option<MmioDeviceEvent> {
        match self.mode {
            ByteMmioDeviceMode::Plain => None,
            ByteMmioDeviceMode::EdgeInterrupt => {
                if self.edge_interrupt_pending {
                    self.edge_interrupt_pending = false;
                    Some(MmioDeviceEvent::InterruptRequested)
                } else {
                    None
                }
            }
            ByteMmioDeviceMode::LevelInterrupt => {
                if self.level_interrupt_pending && !self.level_line_asserted {
                    self.level_line_asserted = true;
                    Some(MmioDeviceEvent::InterruptLineAsserted)
                } else if !self.level_interrupt_pending && self.level_line_asserted {
                    self.level_line_asserted = false;
                    Some(MmioDeviceEvent::InterruptLineDeasserted)
                } else {
                    None
                }
            }
        }
    }

    fn writes(&self) -> &[u8] {
        &self.writes
    }
}

fn exact_write_byte(exit: &MmioExit) -> Result<u8, MmioError> {
    if exit.write_data().len() != 1 {
        return Err(MmioError::InvalidWritePayload {
            address: exit.address(),
            expected: 1,
            actual: exit.write_data().len(),
        });
    }
    Ok(exit.write_data()[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit_at(address: u64, direction: MmioDirection, length: u32, write_data: &[u8]) -> MmioExit {
        MmioExit::new_for_test(address, direction, length, write_data.to_vec())
    }

    fn exit(direction: MmioDirection, length: u32, write_data: &[u8]) -> MmioExit {
        exit_at(BYTE_DEVICE_ADDRESS, direction, length, write_data)
    }

    #[test]
    fn byte_device_captures_one_byte_write() {
        let mut bus = MmioBus::with_byte_device(b'R');
        assert_eq!(
            bus.dispatch(&exit(MmioDirection::Write, 1, b"W")).unwrap(),
            MmioService::Write
        );
        assert_eq!(bus.writes(), Some(&b"W"[..]));
        assert_eq!(bus.take_device_event(), None);
    }

    #[test]
    fn interrupting_byte_device_write_owns_one_consumable_event() {
        let mut bus = MmioBus::with_interrupting_byte_device_at(BYTE_DEVICE_ADDRESS, b'R');
        assert_eq!(
            bus.dispatch(&exit(MmioDirection::Write, 1, b"W")).unwrap(),
            MmioService::Write
        );
        assert_eq!(bus.writes(), Some(&b"W"[..]));
        assert_eq!(
            bus.take_device_event(),
            Some(MmioDeviceEvent::InterruptRequested)
        );
        assert_eq!(bus.take_device_event(), None);

        assert_eq!(
            bus.dispatch(&exit(MmioDirection::Write, 1, b"X")).unwrap(),
            MmioService::Write
        );
        assert_eq!(
            bus.take_device_event(),
            Some(MmioDeviceEvent::InterruptRequested)
        );
        assert_eq!(bus.take_device_event(), None);
    }

    #[test]
    fn interrupting_byte_device_reads_and_invalid_accesses_do_not_request_interrupts() {
        let mut bus = MmioBus::with_interrupting_byte_device_at(BYTE_DEVICE_ADDRESS, b'R');
        assert_eq!(
            bus.dispatch(&exit(MmioDirection::Read, 1, &[])).unwrap(),
            MmioService::Read(vec![b'R'])
        );
        assert_eq!(bus.take_device_event(), None);

        assert!(bus.dispatch(&exit(MmioDirection::Write, 2, b"W")).is_err());
        assert_eq!(bus.take_device_event(), None);
        assert!(bus.dispatch(&exit(MmioDirection::Write, 1, b"AB")).is_err());
        assert_eq!(bus.take_device_event(), None);
    }

    #[test]
    fn level_interrupt_device_tracks_command_status_ack_and_line_transitions() {
        let base = 0x1000_0000;
        let mut bus = MmioBus::with_level_interrupt_byte_device_at(base);

        assert_eq!(
            bus.dispatch(&exit_at(base, MmioDirection::Write, 1, b"W"))
                .unwrap(),
            MmioService::Write
        );
        assert_eq!(
            bus.take_device_event(),
            Some(MmioDeviceEvent::InterruptLineAsserted)
        );
        assert_eq!(bus.take_device_event(), None);
        assert_eq!(
            bus.dispatch(&exit_at(
                base + LEVEL_INTERRUPT_STATUS_OFFSET,
                MmioDirection::Read,
                1,
                &[]
            ))
            .unwrap(),
            MmioService::Read(vec![LEVEL_INTERRUPT_STATUS_PENDING])
        );
        assert_eq!(bus.take_device_event(), None);

        assert_eq!(
            bus.dispatch(&exit_at(
                base + LEVEL_INTERRUPT_ACK_OFFSET,
                MmioDirection::Write,
                1,
                &[1]
            ))
            .unwrap(),
            MmioService::Write
        );
        assert_eq!(
            bus.take_device_event(),
            Some(MmioDeviceEvent::InterruptLineDeasserted)
        );
        assert_eq!(bus.take_device_event(), None);
        assert_eq!(
            bus.dispatch(&exit_at(
                base + LEVEL_INTERRUPT_STATUS_OFFSET,
                MmioDirection::Read,
                1,
                &[]
            ))
            .unwrap(),
            MmioService::Read(vec![0])
        );
        assert_eq!(bus.writes(), Some(&[b'W', 1][..]));
    }

    #[test]
    fn level_interrupt_device_coalesces_repeated_commands_until_ack() {
        let base = 0x1000_0000;
        let mut bus = MmioBus::with_level_interrupt_byte_device_at(base);
        for value in [b'W', b'X'] {
            assert_eq!(
                bus.dispatch(&exit_at(base, MmioDirection::Write, 1, &[value]))
                    .unwrap(),
                MmioService::Write
            );
        }
        assert_eq!(
            bus.take_device_event(),
            Some(MmioDeviceEvent::InterruptLineAsserted)
        );
        assert_eq!(bus.take_device_event(), None);
    }

    #[test]
    fn level_interrupt_device_rejects_wrong_register_directions_and_widths() {
        let base = 0x1000_0000;
        let mut bus = MmioBus::with_level_interrupt_byte_device_at(base);
        assert!(bus
            .dispatch(&exit_at(base, MmioDirection::Read, 1, &[]))
            .is_err());
        assert!(bus
            .dispatch(&exit_at(
                base + LEVEL_INTERRUPT_STATUS_OFFSET,
                MmioDirection::Write,
                1,
                &[1]
            ))
            .is_err());
        assert!(bus
            .dispatch(&exit_at(
                base + LEVEL_INTERRUPT_ACK_OFFSET,
                MmioDirection::Read,
                1,
                &[]
            ))
            .is_err());
        assert!(bus
            .dispatch(&exit_at(base, MmioDirection::Write, 2, b"WW"))
            .is_err());
        assert_eq!(bus.take_device_event(), None);
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
    fn configured_byte_device_address_is_exact() {
        let address = 0x1000_0000;
        let mut bus = MmioBus::with_byte_device_at(address, b'R');
        assert_eq!(
            bus.dispatch(&exit_at(address, MmioDirection::Read, 1, &[]))
                .unwrap(),
            MmioService::Read(vec![b'R'])
        );
        assert!(matches!(
            bus.dispatch(&exit_at(BYTE_DEVICE_ADDRESS, MmioDirection::Read, 1, &[])),
            Err(Error::Mmio(MmioError::UnhandledAddress {
                address: BYTE_DEVICE_ADDRESS,
                ..
            }))
        ));
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
