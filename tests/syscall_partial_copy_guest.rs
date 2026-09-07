use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::syscall::partial_dispatch::{
    run_partial_dispatch_guest, PARTIAL_BYTE_VALUE, PARTIAL_COMMON_FIXUP_RIP,
    PARTIAL_DEST_FAULT_ADDR, PARTIAL_DEST_FAULT_BYTES, PARTIAL_EINVAL, PARTIAL_ENOSYS,
    PARTIAL_GOOD_BYTES, PARTIAL_PROOF, PARTIAL_READ_FAULT_RIP, PARTIAL_SHORT_BYTES,
    PARTIAL_SOURCE_FAULT_ADDR, PARTIAL_SOURCE_FAULT_BYTES, PARTIAL_WRITE_FAULT_RIP,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

const X86_RFLAGS_RESERVED: u64 = 1 << 1;
const X86_RFLAGS_IF: u64 = 1 << 9;
const PAGE_FAULT_SAVED_RFLAGS: u64 = 0x10046;

#[test]
fn bounded_copy_syscall_reports_partial_progress_and_preserves_dispatcher_services() {
    match run_partial_dispatch_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(
                result.returns(),
                [
                    4,
                    2,
                    2,
                    1,
                    PARTIAL_EINVAL,
                    PARTIAL_EINVAL,
                    0,
                    0,
                    PARTIAL_ENOSYS
                ]
            );
            assert_eq!(result.good_destination(), PARTIAL_GOOD_BYTES);
            assert_eq!(
                result.source_fault_destination(),
                [
                    PARTIAL_SOURCE_FAULT_BYTES[0],
                    PARTIAL_SOURCE_FAULT_BYTES[1],
                    0,
                    0
                ]
            );
            assert_eq!(
                result.destination_fault_destination(),
                [
                    PARTIAL_DEST_FAULT_BYTES[0],
                    PARTIAL_DEST_FAULT_BYTES[1],
                    0,
                    0
                ]
            );
            assert_eq!(
                result.short_destination(),
                [PARTIAL_SHORT_BYTES[0], 0, 0, 0]
            );
            assert_eq!(result.byte_destination(), PARTIAL_BYTE_VALUE);

            let read = result.read_fault();
            assert_eq!(read.cr2(), PARTIAL_SOURCE_FAULT_ADDR);
            assert_eq!(read.error_code(), 0);
            assert_eq!(read.rip(), PARTIAL_READ_FAULT_RIP);
            assert_eq!(read.rflags(), PAGE_FAULT_SAVED_RFLAGS);
            assert_eq!(read.resolved_fixup(), PARTIAL_COMMON_FIXUP_RIP);

            let write = result.write_fault();
            assert_eq!(write.cr2(), PARTIAL_DEST_FAULT_ADDR);
            assert_eq!(write.error_code(), 0x2);
            assert_eq!(write.rip(), PARTIAL_WRITE_FAULT_RIP);
            assert_eq!(write.rflags(), PAGE_FAULT_SAVED_RFLAGS);
            assert_eq!(write.resolved_fixup(), PARTIAL_COMMON_FIXUP_RIP);

            assert_eq!(result.fixup_entries().len(), 2);
            assert_eq!(result.proof(), PARTIAL_PROOF);
            assert_eq!(result.io_exits().len(), PARTIAL_PROOF.len());
            for (exit, expected) in result.io_exits().iter().zip(PARTIAL_PROOF.iter().copied()) {
                assert_eq!(exit.direction(), PortIoDirection::Out);
                assert_eq!(exit.size(), 1);
                assert_eq!(exit.count(), 1);
                assert_eq!(exit.output_data(), &[expected]);
            }

            for (_, pte) in result.user_page_ptes() {
                assert_eq!(pte & 0x6, 0x6);
            }
            assert_eq!(result.service_pte() & 0x4, 0);
            assert_eq!(result.fault_handler_pte() & 0x4, 0);
            assert_eq!(result.fault_metadata_pte() & 0x4, 0);

            assert_eq!(
                result.terminal_frame().rflags() & X86_RFLAGS_IF,
                X86_RFLAGS_IF
            );
            assert_eq!(result.terminal_rflags() & X86_RFLAGS_IF, 0);
            assert_eq!(
                result.terminal_rflags() & X86_RFLAGS_RESERVED,
                X86_RFLAGS_RESERVED
            );
            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(
                result.report().rflags() & X86_RFLAGS_RESERVED,
                X86_RFLAGS_RESERVED
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping partial-copy syscall integration assertion: /dev/kvm unavailable")
        }
        Err(error) => panic!("partial-copy syscall guest execution failed unexpectedly: {error}"),
    }
}
