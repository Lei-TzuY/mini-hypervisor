use mini_hypervisor::portio::pci::virtio_blk::{
    run_file_backed_identity_pin_proof, FILE_BACKED_IDENTITY_PIN_PROOF,
};

#[test]
fn file_backed_device_pins_open_host_file_identity_across_path_replacement() {
    let result =
        run_file_backed_identity_pin_proof().expect("pinned file-backed identity proof must succeed");

    assert_eq!(result.write_completion().descriptor_id(), 0);
    assert_eq!(result.write_completion().length(), 1);
    assert_eq!(result.write_completion().sector(), 0);
    assert_ne!(result.original_identity(), result.replacement_identity());
    assert_eq!(result.pinned_sector(), result.payload());
    assert_ne!(result.replacement_sector(), result.payload());
    assert!(result.checkpoint_rejected());
    assert_eq!(result.proof(), FILE_BACKED_IDENTITY_PIN_PROOF);
}
