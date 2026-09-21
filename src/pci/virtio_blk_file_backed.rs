use super::*;
use crate::error::{Error, HostEnvironmentError};
use crate::memory::{GuestMemory, GuestPhysAddr};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::ops::Range;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub const FILE_BACKED_VIRTIO_BLK_PROOF: &[u8; 5] = b"FWSDR";
pub const FILE_BACKED_IDENTITY_PIN_PROOF: &[u8; 5] = b"FPINW";

const PROOF_BAR: u64 = 0x1000_0000;
const PROOF_DESC: u64 = 0x18000;
const PROOF_AVAIL: u64 = 0x18100;
const PROOF_USED: u64 = 0x18200;
const PROOF_HEADER: u64 = 0x18300;
const PROOF_DATA: u64 = 0x18400;
const PROOF_STATUS: u64 = 0x18600;
const PROOF_MEMORY_SIZE: u64 = 0x20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioBlkFileIdentity {
    device_id: u64,
    inode: u64,
}

impl VirtioBlkFileIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device_id: metadata.dev(),
            inode: metadata.ino(),
        }
    }

    #[must_use]
    pub const fn device_id(self) -> u64 {
        self.device_id
    }

    #[must_use]
    pub const fn inode(self) -> u64 {
        self.inode
    }
}

#[derive(Clone)]
pub(super) struct PersistentFileBacking {
    origin_path: PathBuf,
    file: Arc<File>,
    identity: VirtioBlkFileIdentity,
}

impl PersistentFileBacking {
    fn from_open_file(origin_path: PathBuf, file: File) -> Result<Self, Error> {
        let metadata = file
            .metadata()
            .map_err(|source| backing_io_error("stat pinned virtio-blk file backing", source))?;
        Ok(Self {
            origin_path,
            file: Arc::new(file),
            identity: VirtioBlkFileIdentity::from_metadata(&metadata),
        })
    }

    fn write_all_at_and_sync(&self, offset: u64, bytes: &[u8]) -> Result<(), Error> {
        let mut written = 0_usize;
        while written < bytes.len() {
            match self.file.write_at(
                &bytes[written..],
                offset + u64::try_from(written).expect("bounded backing offset fits u64"),
            ) {
                Ok(0) => {
                    return Err(backing_io_error(
                        "write pinned virtio-blk file backing",
                        std::io::Error::new(
                            std::io::ErrorKind::WriteZero,
                            "host file accepted zero bytes before the range was complete",
                        ),
                    ))
                }
                Ok(count) => written += count,
                Err(source) if source.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(source) => {
                    return Err(backing_io_error(
                        "write pinned virtio-blk file backing",
                        source,
                    ))
                }
            }
        }
        self.file
            .sync_all()
            .map_err(|source| backing_io_error("sync pinned virtio-blk file backing write", source))
    }
}

impl std::fmt::Debug for PersistentFileBacking {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PersistentFileBacking")
            .field("origin_path", &self.origin_path)
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl PartialEq for PersistentFileBacking {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl Eq for PersistentFileBacking {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtioBlkFileIdentityPinProof {
    write_completion: VirtioBlkQueueCompletion,
    payload: [u8; VIRTIO_BLK_SECTOR_SIZE],
    original_identity: VirtioBlkFileIdentity,
    replacement_identity: VirtioBlkFileIdentity,
    pinned_sector: [u8; VIRTIO_BLK_SECTOR_SIZE],
    replacement_sector: [u8; VIRTIO_BLK_SECTOR_SIZE],
    checkpoint_rejected: bool,
    proof: Vec<u8>,
}

impl VirtioBlkFileIdentityPinProof {
    #[must_use]
    pub const fn write_completion(&self) -> VirtioBlkQueueCompletion {
        self.write_completion
    }

    #[must_use]
    pub const fn payload(&self) -> &[u8; VIRTIO_BLK_SECTOR_SIZE] {
        &self.payload
    }

    #[must_use]
    pub const fn original_identity(&self) -> VirtioBlkFileIdentity {
        self.original_identity
    }

    #[must_use]
    pub const fn replacement_identity(&self) -> VirtioBlkFileIdentity {
        self.replacement_identity
    }

    #[must_use]
    pub const fn pinned_sector(&self) -> &[u8; VIRTIO_BLK_SECTOR_SIZE] {
        &self.pinned_sector
    }

    #[must_use]
    pub const fn replacement_sector(&self) -> &[u8; VIRTIO_BLK_SECTOR_SIZE] {
        &self.replacement_sector
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
        let persistent_backing = PersistentFileBacking::from_open_file(path, file)?;
        Ok(Self::with_backing(
            bar0,
            backing,
            Some(persistent_backing),
        ))
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
        let persistent_backing = PersistentFileBacking::from_open_file(path, file)?;
        Ok(Self::with_backing(
            bar0,
            backing,
            Some(persistent_backing),
        ))
    }

    fn with_backing(
        bar0: u64,
        backing: [u8; VIRTIO_BLK_BACKING_SIZE],
        persistent_backing: Option<PersistentFileBacking>,
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

    #[must_use]
    pub fn file_backing_identity(&self) -> Option<VirtioBlkFileIdentity> {
        self.persistent_backing
            .as_ref()
            .map(|backing| backing.identity)
    }

    pub(super) fn persist_backing_range(
        &self,
        range: Range<usize>,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let Some(backing) = self.persistent_backing.as_ref() else {
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
        backing.write_all_at_and_sync(range.start as u64, bytes)
    }

    #[must_use]
    pub(crate) const fn checkpoint_backing_portable(&self) -> bool {
        self.persistent_backing.is_none()
    }
}

pub fn run_file_backed_identity_pin_proof() -> Result<VirtioBlkFileIdentityPinProof, Error> {
    let path = temporary_backing_path();
    let pinned_path = path.with_extension("pinned.img");
    let _path_cleanup = BackingCleanup(path.clone());
    let _pinned_cleanup = BackingCleanup(pinned_path.clone());

    let payload = deterministic_identity_payload();
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), PROOF_MEMORY_SIZE)?;
    let mut device = VirtioBlkDevice::create_file_backed(PROOF_BAR, &path)?;
    prepare_ready_device(&mut device);
    let original_identity = device
        .file_backing_identity()
        .ok_or_else(|| proof_error("file-backed device lost its pinned host-file identity"))?;

    fs::rename(&path, &pinned_path)
        .map_err(|source| backing_io_error("rename pinned virtio-blk backing", source))?;
    let pinned_identity = file_identity_at_path(&pinned_path)?;
    if pinned_identity != original_identity {
        return Err(proof_error(format!(
            "renamed backing identity changed from {original_identity:?} to {pinned_identity:?}"
        )));
    }

    let replacement = VirtioBlkDevice::create_file_backed(PROOF_BAR + 0x1000, &path)?;
    let replacement_identity = replacement
        .file_backing_identity()
        .ok_or_else(|| proof_error("replacement file-backed device lost host-file identity"))?;
    drop(replacement);
    if replacement_identity == original_identity {
        return Err(proof_error(
            "replacement path unexpectedly resolved to the already-open backing identity",
        ));
    }

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

    if device.file_backing_identity() != Some(original_identity) {
        return Err(proof_error(
            "device changed pinned host-file identity after origin path replacement",
        ));
    }

    let pinned_raw = fs::read(&pinned_path)
        .map_err(|source| backing_io_error("read renamed pinned virtio-blk backing", source))?;
    let replacement_raw = fs::read(&path)
        .map_err(|source| backing_io_error("read replacement virtio-blk backing", source))?;
    if pinned_raw.len() != VIRTIO_BLK_BACKING_SIZE
        || replacement_raw.len() != VIRTIO_BLK_BACKING_SIZE
    {
        return Err(proof_error(
            "pinned or replacement file changed bounded backing length",
        ));
    }

    let mut pinned_sector = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
    pinned_sector.copy_from_slice(&pinned_raw[..VIRTIO_BLK_SECTOR_SIZE]);
    let mut replacement_sector = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
    replacement_sector.copy_from_slice(&replacement_raw[..VIRTIO_BLK_SECTOR_SIZE]);
    let initial_sector = virtio_blk_backing::deterministic_backing();
    if pinned_sector != payload {
        return Err(proof_error(
            "guest T_OUT did not update the originally opened backing after path replacement",
        ));
    }
    if replacement_sector != initial_sector[..VIRTIO_BLK_SECTOR_SIZE] {
        return Err(proof_error(
            "guest T_OUT was redirected into the replacement pathname backing",
        ));
    }

    let checkpoint_rejected = matches!(
        VirtioBlkCheckpointState::capture(&device),
        Err(VirtioBlkCheckpointStateError::ExternalBackingUnsupported)
    );
    if !checkpoint_rejected {
        return Err(proof_error(
            "pinned file-backed device checkpoint did not fail closed",
        ));
    }

    Ok(VirtioBlkFileIdentityPinProof {
        write_completion,
        payload,
        original_identity,
        replacement_identity,
        pinned_sector,
        replacement_sector,
        checkpoint_rejected,
        proof: FILE_BACKED_IDENTITY_PIN_PROOF.to_vec(),
    })
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

fn deterministic_identity_payload() -> [u8; VIRTIO_BLK_SECTOR_SIZE] {
    let mut bytes = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(29).wrapping_add(0x31);
    }
    bytes[..16].copy_from_slice(b"FILE-PIN-000000!");
    bytes[VIRTIO_BLK_SECTOR_SIZE - 8..].copy_from_slice(b"PINNED!!");
    bytes
}

fn file_identity_at_path(path: &Path) -> Result<VirtioBlkFileIdentity, Error> {
    let metadata = fs::metadata(path)
        .map_err(|source| backing_io_error("stat virtio-blk backing identity", source))?;
    Ok(VirtioBlkFileIdentity::from_metadata(&metadata))
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
    fn replacement_path_does_not_redirect_pinned_backend() {
        let proof = run_file_backed_identity_pin_proof().unwrap();
        assert_ne!(proof.original_identity(), proof.replacement_identity());
        assert_eq!(proof.pinned_sector(), proof.payload());
        assert_ne!(proof.replacement_sector(), proof.payload());
        assert!(proof.checkpoint_rejected());
        assert_eq!(proof.proof(), FILE_BACKED_IDENTITY_PIN_PROOF);
    }

    #[test]
    fn file_backed_reopen_proof_round_trips_synced_payload() {
        let proof = run_file_backed_reopen_proof().unwrap();
        assert_eq!(proof.persisted_sector(), proof.readback());
        assert!(proof.checkpoint_rejected());
        assert_eq!(proof.proof(), FILE_BACKED_VIRTIO_BLK_PROOF);
    }
}
