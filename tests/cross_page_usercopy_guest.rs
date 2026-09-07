use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::privilege::{
    PRIVILEGE_TSS_RSP0, PRIVILEGE_USER_CODE_SELECTOR, PRIVILEGE_USER_DATA_SELECTOR,
    PRIVILEGE_USER_STACK,
};
use mini_hypervisor::syscall::cross_page::{
    run_cross_page_usercopy_guest, CROSS_PAGE_COMMON_FIXUP_RIP, CROSS_PAGE_COPY_LEN,
    CROSS_PAGE_DEST_FAULT_ADDR, CROSS_PAGE_DEST_FAULT_BYTES, CROSS_PAGE_DEST_FAULT_DESTINATION,
    CROSS_PAGE_GOOD_BYTES, CROSS_PAGE_PROOF, CROSS_PAGE_READ_FAULT_RIP,
    CROSS_PAGE_SOURCE_FAULT_ADDR, CROSS_PAGE_SOURCE_FAULT_BYTES, CROSS_PAGE_WRITE_FAULT_RIP,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
const KERNEL_CODE_SELECTOR: u64 = 0x08;
const PAGE_FAULT_SAVED_RFLAGS: u64 = 0x1_0046;
const TERMINAL_USER_RIP: u64 = 0x1_105f;
const TERMINAL_KERNEL_RSP: u64 = PRIVILEGE_TSS_RSP0 - 5 * 8;

#[test]
fn cross_page_copy_reports_exact_partial_progress_after_read_and_write_faults() {
    match run_cross_page_usercopy_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.proof(), CROSS_PAGE_PROOF);
            assert_eq!(result.returns(), [CROSS_PAGE_COPY_LEN, 2, 2]);
            assert_eq!(result.good_source(), CROSS_PAGE_GOOD_BYTES);
            assert_eq!(result.good_destination(), CROSS_PAGE_GOOD_BYTES);
            assert_eq!(result.source_fault_source(), CROSS_PAGE_SOURCE_FAULT_BYTES);
            assert_eq!(
                result.source_fault_destination(),
                [CROSS_PAGE_SOURCE_FAULT_BYTES[0], CROSS_PAGE_SOURCE_FAULT_BYTES[1], 0, 0]
            );
            assert_eq!(result.destination_fault_source(), CROSS_PAGE_DEST_FAULT_BYTES);
            assert_eq!(
                result.destination_fault_destination(),
                [CROSS_PAGE_DEST_FAULT_BYTES[0], CROSS_PAGE_DEST_FAULT_BYTES[1], 0, 0]
            );

            let read = result.read_fault();
            assert_eq!(read.cr2(), CROSS_PAGE_SOURCE_FAULT_ADDR);
            assert_eq!(read.error_code(), 0);
            assert_eq!(read.rip(), CROSS_PAGE_READ_FAULT_RIP);
            assert_eq!(read.cs(), KERNEL_CODE_SELECTOR);
            assert_eq!(read.rflags(), PAGE_FAULT_SAVED_RFLAGS);
            assert_eq!(read.resolved_fixup(), CROSS_PAGE_COMMON_FIXUP_RIP);

            let write = result.write_fault();
            assert_eq!(write.cr2(), CROSS_PAGE_DEST_FAULT_ADDR);
            assert_eq!(write.error_code(), 0x2);
            assert_eq!(write.rip(), CROSS_PAGE_WRITE_FAULT_RIP);
            assert_eq!(write.cs(), KERNEL_CODE_SELECTOR);
            assert_eq!(write.rflags(), PAGE_FAULT_SAVED_RFLAGS);
            assert_eq!(write.resolved_fixup(), CROSS_PAGE_COMMON_FIXUP_RIP);
            assert_eq!(result.final_cr2(), CROSS_PAGE_DEST_FAULT_ADDR);

            let fixups = result.fixup_entries();
            assert_eq!(fixups.len(), 2);
            assert_eq!(fixups[0].fault_rip(), CROSS_PAGE_READ_FAULT_RIP);
            assert_eq!(fixups[0].fixup_rip(), CROSS_PAGE_COMMON_FIXUP_RIP);
            assert_eq!(fixups[1].fault_rip(), CROSS_PAGE_WRITE_FAULT_RIP);
            assert_eq!(fixups[1].fixup_rip(), CROSS_PAGE_COMMON_FIXUP_RIP);

            let mappings = result.user_page_ptes();
            assert_eq!(mappings.len(), 12);
            for &(address, pte) in mappings {
                assert_eq!(pte & (X86_PAGE_WRITE | X86_PAGE_USER), X86_PAGE_WRITE | X86_PAGE_USER);
                let expected_present = address != CROSS_PAGE_SOURCE_FAULT_ADDR
                    && address != CROSS_PAGE_DEST_FAULT_ADDR;
                assert_eq!(pte & X86_PAGE_PRESENT != 0, expected_present);
            }
            assert_eq!(result.service_pte() & X86_PAGE_USER, 0);
            assert_eq!(result.fault_handler_pte() & X86_PAGE_USER, 0);
            assert_eq!(result.fault_metadata_pte() & X86_PAGE_USER, 0);

            let frame = result.terminal_frame();
            assert_eq!(frame.rip(), TERMINAL_USER_RIP);
            assert_eq!(frame.cs(), u64::from(PRIVILEGE_USER_CODE_SELECTOR));
            assert_eq!(frame.rflags(), 0x202);
            assert_eq!(frame.rsp(), PRIVILEGE_USER_STACK);
            assert_eq!(frame.ss(), u64::from(PRIVILEGE_USER_DATA_SELECTOR));
            assert_eq!(result.terminal_rsp(), TERMINAL_KERNEL_RSP);
            assert_eq!(result.terminal_cs(), KERNEL_CODE_SELECTOR as u16);
            assert_eq!(result.terminal_rflags() & 0x2, 0x2);
            assert_eq!(result.terminal_rflags() & 0x200, 0);
            assert_eq!(result.report().exit(), VcpuExit::Hlt);

            assert_eq!(result.io_exits().len(), CROSS_PAGE_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(CROSS_PAGE_PROOF.iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            // This is a host backing-memory sanity check only; the architectural destination fault
            // itself is proven by CR2/error/RIP/fixup and the return value of two completed bytes.
            assert_eq!(CROSS_PAGE_DEST_FAULT_DESTINATION + 2, CROSS_PAGE_DEST_FAULT_ADDR);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping cross-page usercopy integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("cross-page usercopy execution failed unexpectedly: {error}"),
    }
}
