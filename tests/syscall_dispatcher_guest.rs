use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::privilege::{
    PRIVILEGE_TSS_RSP0, PRIVILEGE_USER_CODE_SELECTOR, PRIVILEGE_USER_DATA_SELECTOR,
    PRIVILEGE_USER_STACK,
};
use mini_hypervisor::syscall::dispatcher::{
    run_syscall_dispatch_guest, DISPATCH_BAD_POINTER, DISPATCH_COPY_READ_FAULT_RIP,
    DISPATCH_COPY_READ_FIXUP_RIP, DISPATCH_COPY_WRITE_FAULT_RIP, DISPATCH_COPY_WRITE_FIXUP_RIP,
    DISPATCH_EFAULT, DISPATCH_ENOSYS, DISPATCH_PROOF, DISPATCH_READ_FAULT_OBSERVATION_ADDR,
    DISPATCH_TERMINAL_HLT_RIP, DISPATCH_TERMINAL_RETURN_RIP, DISPATCH_VALUE,
    DISPATCH_WRITE_FAULT_OBSERVATION_ADDR,
};
use mini_hypervisor::syscall::{
    EFER_SYSCALL_ENABLE, SYSCALL_LSTAR_VALUE, SYSCALL_SFMASK_VALUE, SYSCALL_STAR_VALUE,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_RFLAGS_RF: u64 = 1 << 16;

#[test]
fn syscall_number_dispatches_copy_putc_faults_and_unknown_service() {
    match run_syscall_dispatch_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.proof(), DISPATCH_PROOF);
            assert_eq!(result.io_exits().len(), DISPATCH_PROOF.len());
            for (io, expected) in result.io_exits().iter().zip(DISPATCH_PROOF.iter().copied()) {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.size(), 1);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            assert_eq!(result.copy_return(), 0);
            assert_eq!(result.copy_readback(), u64::from(DISPATCH_VALUE));
            assert_eq!(result.putc_return(), 0);
            assert_eq!(result.bad_source_return(), DISPATCH_EFAULT);
            assert_eq!(result.bad_destination_return(), DISPATCH_EFAULT);
            assert_eq!(result.unknown_return(), DISPATCH_ENOSYS);
            assert_eq!(result.source_value(), DISPATCH_VALUE);
            assert_eq!(result.destination_value(), DISPATCH_VALUE);

            let read = result.read_fault();
            assert_eq!(read.cr2(), DISPATCH_BAD_POINTER);
            assert_eq!(read.error_code(), 0);
            assert_eq!(read.rip(), DISPATCH_COPY_READ_FAULT_RIP);
            assert_eq!(read.cs(), 0x08);
            assert_eq!(read.rflags(), 0x2 | X86_RFLAGS_RF);
            assert_eq!(read.resolved_fixup(), DISPATCH_COPY_READ_FIXUP_RIP);

            let write = result.write_fault();
            assert_eq!(write.cr2(), DISPATCH_BAD_POINTER);
            assert_eq!(write.error_code(), 0x2);
            assert_eq!(write.rip(), DISPATCH_COPY_WRITE_FAULT_RIP);
            assert_eq!(write.cs(), 0x08);
            assert_eq!(write.rflags(), 0x2 | X86_RFLAGS_RF);
            assert_eq!(write.resolved_fixup(), DISPATCH_COPY_WRITE_FIXUP_RIP);
            assert_eq!(result.final_cr2(), DISPATCH_BAD_POINTER);

            let table = result.fixup_entries();
            assert_eq!(table[0].fault_rip(), DISPATCH_COPY_READ_FAULT_RIP);
            assert_eq!(table[0].fixup_rip(), DISPATCH_COPY_READ_FIXUP_RIP);
            assert_eq!(
                table[0].observation_addr(),
                DISPATCH_READ_FAULT_OBSERVATION_ADDR.get()
            );
            assert_eq!(table[1].fault_rip(), DISPATCH_COPY_WRITE_FAULT_RIP);
            assert_eq!(table[1].fixup_rip(), DISPATCH_COPY_WRITE_FIXUP_RIP);
            assert_eq!(
                table[1].observation_addr(),
                DISPATCH_WRITE_FAULT_OBSERVATION_ADDR.get()
            );

            let frame = result.terminal_frame();
            assert_eq!(frame.rip(), DISPATCH_TERMINAL_RETURN_RIP);
            assert_eq!(frame.cs(), u64::from(PRIVILEGE_USER_CODE_SELECTOR));
            assert_eq!(frame.rflags(), 0x202);
            assert_eq!(frame.rsp(), PRIVILEGE_USER_STACK);
            assert_eq!(frame.ss(), u64::from(PRIVILEGE_USER_DATA_SELECTOR));

            assert_ne!(result.efer() & EFER_SYSCALL_ENABLE, 0);
            assert_eq!(result.star(), SYSCALL_STAR_VALUE);
            assert_eq!(result.lstar(), SYSCALL_LSTAR_VALUE);
            assert_eq!(result.sfmask(), SYSCALL_SFMASK_VALUE);
            assert_eq!(
                result.user_page_pte() & (X86_PAGE_USER | X86_PAGE_WRITE),
                X86_PAGE_USER | X86_PAGE_WRITE
            );
            assert_eq!(result.dispatcher_pte() & X86_PAGE_USER, 0);
            assert_eq!(result.fault_handler_pte() & X86_PAGE_USER, 0);
            assert_eq!(result.fault_metadata_pte() & X86_PAGE_USER, 0);
            assert_eq!(result.bad_pd_entry() & X86_PAGE_PRESENT, 0);

            assert_eq!(result.terminal_rsp(), PRIVILEGE_TSS_RSP0 - 40);
            assert_eq!(result.terminal_cs(), 0x08);
            assert_eq!(result.terminal_rflags() & 0x2, 0x2);
            assert_eq!(result.terminal_rflags() & 0x200, 0);
            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rip(), DISPATCH_TERMINAL_HLT_RIP);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping syscall dispatcher integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("syscall dispatcher guest execution failed unexpectedly: {error}"),
    }
}
