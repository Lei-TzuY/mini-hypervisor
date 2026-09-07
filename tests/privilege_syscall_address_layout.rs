use mini_hypervisor::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
use mini_hypervisor::privilege::PRIVILEGE_RETURN_HANDLER;
use mini_hypervisor::syscall::SYSCALL_KERNEL_ENTRY;

#[test]
fn privilege_return_handler_does_not_overlap_syscall_entry() {
    assert_ne!(PRIVILEGE_RETURN_HANDLER, SYSCALL_KERNEL_ENTRY);
    assert!(PRIVILEGE_RETURN_HANDLER.get() < LONG_MODE_IDENTITY_MAP_SIZE);
}
