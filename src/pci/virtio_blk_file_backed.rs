use super::*;
use crate::error::{Error, HostEnvironmentError};
use crate::memory::{GuestMemory, GuestPhysAddr};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const FILE_BACKED_VIRTIO_BLK_PROOF: &[u8; 5] = b"FWSDR";

const PROOF_BAR: u64 = 0x1000_0000;
const PROOF_DESC: u64 = 0x18000;
const PROOF_AVAIL: u64 = 0x18100;
const PROOF_USED: u64 = 0x18200;
const PROOF_HEADER: u64 = 0x18300;
const PROOF_DATA: u64 = 0x18400;
const PROOF_STATUS: u64 = 0x18600;
const PROOF_MEMORY_SIZE: u64 = 0x20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtioBlkFileBackedProof {
    write_completion: VirtioBlkQueueCompletion,
    read_completion: VirtioBlkQueueCompletion,
    persisted_sector: [u8; VIRTIO_BLK_SECTOR_SIZE],
    readback: [u8; VIRTIO_BLK_SECTOR_SIZE],
    checkpoint_rejected: bool,
    proof: Vec<u8>,
}

impl VirtioBlkFileBackedProof {
    #[must_use]
    pub const fn write_completion(&self) -> VirtioBlkQueueCompletion {
        self.write_completion
    }

    #[must_use]
    pub const fn read_completion(&self) -> VirtioBlkQueueCompletion {
        self.read_completion
    }

    #[must_use]
    pub const fn persisted_sector(&self) -> &[u8; VIRTIO_BLK_SECTOR_SIZE] {
        &self.persisted_sector
    }

    #[must_use]
    pub const fn readback(&self) -> &[u8; VIRTIO_BLK_SECTOR_SIZE] {
        &self.readback
    }

    #[must_use]
    pub const fn checkpoint_rejected(&self) -> bool {
        self.checkpoint_rejected
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
}

impl VirtioBlkDevice {
    pub fn create_file_backed(bar0: u64, path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        let backing = virtio_blk_backing::deterministic_backing();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|source| backing_io_error("create virtio-blk file backing", source))?;
        file.write_all(&backing)
            .map_err(|source| backing_io_error("initialize virtio-blk file backing", source))?;
        file.sync_all()
            .map_err(|source| backing_io_error("sync initial virtio-blk file backing", source))?;
        Ok(Self::with_backing(bar0, backing, Some(path)))
    }

    pub fn open_file_backed(bar0: u64, path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| backing_io_error("open virtio-blk file backing", source))?;
        let length = file
            .metadata()
            .map_err(|source| backing_io_error("stat virtio-blk file backing", source))?
            .len();
        if length != VIRTIO_BLK_BACKING_SIZE as u64 {
            return Err(backing_io_error(
                "validate virtio-blk file backing length",
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "expected exactly {} bytes, got {length}",
                        VIRTIO_BLK_BACKING_SIZE
                    ),
                ),
            ));
        }
        let mut backing = [0_u8; VIRTIO_BLK_BACKING_SIZE];
        file.read_exact(&mut backing)
            .map_err(|source| backing_io_error("read virtio-blk file backing", source))?;
        Ok(Self::with_backing(bar0, backing, Some(path)))
    }

    fn with_backing(
        bar0: u64,
        backing: [u8; VIRTIO_BLK_BACKING_SIZE],
        persistent_backing: Option<PathBuf>,
    ) -> Self {
        let mut device = Self::new(bar0);
        device.backing = backing;
        device.persistent_backing = persistent_backing;
        device
    }

    #[must_use]
    pub const fn file_backed(&self) -> bool {
        self.persistent_backing.is_some()
    }

    pub(super) fn persist_backing_range(
        &self,
        range: Range<usize>,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let Some(path) = self.persistent_backing.as_ref() else {
            return Ok(());
        };
        if range.end > VIRTIO_BLK_BACKING_SIZE || range.len() != bytes.len() {
            return Err(backing_io_error(
                "validate virtio-blk file write range",
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "range {}..{} does not match {} bytes",
                        range.start,
                        range.end,
                        bytes.len()
                    ),
                ),
            ));
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| backing_io_error("open virtio-blk file backing for write", source))?;
        file.seek(SeekFrom::Start(range.start as u64))
            .map_err(|source| backing_io_error("seek virtio-blk file backing", source))?;
        file.write_all(bytes)
            .map_err(|source| backing_io_error("write virtio-blk file backing", source))?;
        file.sync_all()
            .map_err(|source| backing_io_error("sync virtio-blk file backing write", source))
    }

    #[must_use]
    pub(crate) const fn checkpoint_backing_portable(&self) -> bool {
        self.persistent_backing.is_none()
    }
}

pub fn run_file_backed_reopen_proof() -> Result<VirtioBlkFileBackedProof, Error> {
    let path = temporary_backing_path();
    let cleanup = BackingCleanup(path.clone());

    let payload = deterministic_file_payload();
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), PROOF_MEMORY_SIZE)?;
    let mut device = VirtioBlkDevice::create_file_backed(PROOF_BAR, &path)?;
    prepare_ready_device(&mut device);
    prepare_request(
        &mut memory,
        &mut device,
        VIRTIO_BLK_T_OUT,
        VIRTQ_DESC_F_NEXT,
        1,
        &payload,
    )?;
    let write_completion = device
        .process_notified_queue_atomic(&mut memory)
        .map_err(process_error)?;

    let raw = fs::read(&path)
        .map_err(|source| backing_io_error("read synced virtio-blk proof backing", source))?;
    if raw.len() != VIRTIO_BLK_BACKING_SIZE || raw[..VIRTIO_BLK_SECTOR_SIZE] != payload {
        return Err(proof_error(
            "synced host file did not contain the completed T_OUT payload",
        ));
    }
    let mut persisted_sector = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
    persisted_sector.copy_from_slice(&raw[..VIRTIO_BLK_SECTOR_SIZE]);

    let checkpoint_rejected = matches!(
        VirtioBlkCheckpointState::capture(&device),
        Err(VirtioBlkCheckpointStateError::ExternalBackingUnsupported)
    );
    if !checkpoint_rejected {
        return Err(proof_error(
            "file-backed device checkpoint did not fail closed at the storage boundary",
        ));
    }

    drop(device);
    let mut reopened = VirtioBlkDevice::open_file_backed(PROOF_BAR, &path)?;
    prepare_ready_device(&mut reopened);
    memory.write(
        GuestPhysAddr::new(PROOF_DATA),
        &[0x5a; VIRTIO_BLK_SECTOR_SIZE],
    )?;
    prepare_request(
        &mut memory,
        &mut reopened,
        VIRTIO_BLK_T_IN,
        VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
        1,
        &[0; VIRTIO_BLK_SECTOR_SIZE],
    )?;
    let read_completion = reopened
        .process_notified_queue_atomic(&mut memory)
        .map_err(process_error)?;
    let mut readback = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
    memory.read(GuestPhysAddr::new(PROOF_DATA), &mut readback)?;
    if readback != payload || reopened.sector0() != &payload {
        return Err(proof_error(
            "reopened file-backed device did not return the synced T_OUT payload",
        ));
    }

    let proof = FILE_BACKED_VIRTIO_BLK_PROOF.to_vec();
    drop(cleanup);
    Ok(VirtioBlkFileBackedProof {
        write_completion,
        read_completion,
        persisted_sector,
        readback,
        checkpoint_rejected,
        proof,
    })
}

fn prepare_ready_device(device: &mut VirtioBlkDevice) {
    device.driver_features = VIRTIO_F_VERSION_1;
    device.status = VIRTIO_STATUS_ACKNOWLEDGE
        | VIRTIO_STATUS_DRIVER
        | VIRTIO_STATUS_FEATURES_OK
        | VIRTIO_STATUS_DRIVER_OK;
    device.queue_size = 4;
    device.queue_enabled = true;
    device.queue_desc = PROOF_DESC;
    device.queue_driver = PROOF_AVAIL;
    device.queue_device = PROOF_USED;
    device.notify_pending = false;
    device.last_avail_idx = 0;
    device.last_used_idx = 0;
    device.isr_status = 0;
}

fn prepare_request(
    memory: &mut GuestMemory,
    device: &mut VirtioBlkDevice,
    request_type: u32,
    data_flags: u16,
    avail_idx: u16,
    payload: &[u8; VIRTIO_BLK_SECTOR_SIZE],
) -> Result<(), Error> {
    write_descriptor(memory, 0, PROOF_HEADER, 16, VIRTQ_DESC_F_NEXT, 1)?;
    write_descriptor(
        memory,
        1,
        PROOF_DATA,
        VIRTIO_BLK_SECTOR_SIZE as u32,
        data_flags,
        2,
    )?;
    write_descriptor(memory, 2, PROOF_STATUS, 1, VIRTQ_DESC_F_WRITE, 0)?;
    let mut header = [0_u8; 16];
    header[0..4].copy_from_slice(&request_type.to_le_bytes());
    memory.write(GuestPhysAddr::new(PROOF_HEADER), &header)?;
    memory.write(GuestPhysAddr::new(PROOF_DATA), payload)?;
    memory.write(GuestPhysAddr::new(PROOF_STATUS), &[0xff])?;
    memory.write(
        GuestPhysAddr::new(PROOF_AVAIL + 2),
        &avail_idx.to_le_bytes(),
    )?;
    memory.write(GuestPhysAddr::new(PROOF_AVAIL + 4), &0_u16.to_le_bytes())?;
    memory.write(GuestPhysAddr::new(PROOF_USED + 2), &0_u16.to_le_bytes())?;
    device.notify_pending = true;
    Ok(())
}

fn write_descriptor(
    memory: &mut GuestMemory,
    index: u16,
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
) -> Result<(), Error> {
    let mut descriptor = [0_u8; 16];
    descriptor[0..8].copy_from_slice(&address.to_le_bytes());
    descriptor[8..12].copy_from_slice(&length.to_le_bytes());
    descriptor[12..14].copy_from_slice(&flags.to_le_bytes());
    descriptor[14..16].copy_from_slice(&next.to_le_bytes());
    memory.write(
        GuestPhysAddr::new(PROOF_DESC + 16 * u64::from(index)),
        &descriptor,
    )
}

fn deterministic_file_payload() -> [u8; VIRTIO_BLK_SECTOR_SIZE] {
    let mut bytes = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(43).wrapping_add(9);
    }
    bytes[..16].copy_from_slice(b"FILE-BLK-000000!");
    bytes[VIRTIO_BLK_SECTOR_SIZE - 8..].copy_from_slice(b"FSYNCED!");
    bytes
}

fn temporary_backing_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "mini-hypervisor-virtio-blk-{}-{nanos}.img",
        std::process::id()
    ))
}

fn backing_io_error(operation: &'static str, source: std::io::Error) -> Error {
    Error::HostEnvironment(HostEnvironmentError::Io { operation, source })
}

fn proof_error(detail: impl Into<String>) -> Error {
    backing_io_error(
        "verify file-backed virtio-blk persistence proof",
        std::io::Error::new(std::io::ErrorKind::InvalidData, detail.into()),
    )
}

fn process_error(error: VirtioBlkProcessError) -> Error {
    match error {
        VirtioBlkProcessError::Memory(error) | VirtioBlkProcessError::Backing(error) => error,
        VirtioBlkProcessError::Device(error) => proof_error(error.to_string()),
    }
}

struct BackingCleanup(PathBuf);

impl Drop for BackingCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backing_requires_exact_bounded_length() {
        let path = temporary_backing_path();
        fs::write(&path, [0_u8; 1]).unwrap();
        assert!(VirtioBlkDevice::open_file_backed(PROOF_BAR, &path).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn file_backed_reopen_proof_round_trips_synced_payload() {
        let proof = run_file_backed_reopen_proof().unwrap();
        assert_eq!(proof.persisted_sector(), proof.readback());
        assert!(proof.checkpoint_rejected());
        assert_eq!(proof.proof(), FILE_BACKED_VIRTIO_BLK_PROOF);
    }
}
