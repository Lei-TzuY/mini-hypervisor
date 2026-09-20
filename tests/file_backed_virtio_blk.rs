use mini_hypervisor::portio::pci::virtio_blk::{
    run_file_backed_reopen_proof, FILE_BACKED_VIRTIO_BLK_PROOF, VIRTIO_BLK_SECTOR_SIZE,
};

#[test]
fn file_backed_write_is_synced_before_reopen_readback() {
    let result = run_file_backed_reopen_proof().expect("file-backed virtio-blk proof must succeed");
    assert_eq!(result.write_completion().descriptor_id(), 0);
    assert_eq!(result.write_completion().length(), 1);
    assert_eq!(result.write_completion().sector(), 0);
    assert_eq!(result.read_completion().descriptor_id(), 0);
    assert_eq!(
        result.read_completion().length(),
        (VIRTIO_BLK_SECTOR_SIZE + 1) as u32
    );
    assert_eq!(result.read_completion().sector(), 0);
    assert_eq!(result.persisted_sector(), result.readback());
    assert!(result.checkpoint_rejected());
    assert_eq!(result.proof(), FILE_BACKED_VIRTIO_BLK_PROOF);
}
