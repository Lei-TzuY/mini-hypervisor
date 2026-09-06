use mini_hypervisor::config::VmConfig;
use mini_hypervisor::copyout::{
    run_fault_safe_copyout_guest, COPYOUT_BAD_POINTER, COPYOUT_EFAULT, COPYOUT_FAULT_RIP,
    COPYOUT_PROOF, COPYOUT_TERMINAL_HLT_RIP, COPYOUT_TERMINAL_RETURN_RIP, COPYOUT_VALUE,
};
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::privilege::{
    PRIVILEGE_TSS_RSP0, PRIVILEGE_USER_CODE_SELECTOR, PRIVILEGE_USER_DATA_SELECTOR,
    PRIVILEGE_USER_STACK,
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
fn bad_ring3_destination_faults_in_kernel_copyout_and_recovers_to_user() {
    match run_fault_safe_copyout_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.proof(), COPYOUT_PROOF);
            assert_eq!(result.io_exits().len(), COPYOUT_PROOF.len());
            for (io, expected) in result.io_exits().iter().zip(COPYOUT_PROOF.iter().copied()) {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.size(), 1);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            assert_eq!(result.good_return(), 0);
            assert_eq!(result.bad_return(), COPYOUT_EFAULT);
            assert_eq!(result.user_readback(), u64::from(COPYOUT_VALUE));
            assert_eq!(result.user_memory_value(), COPYOUT_VALUE);

            let fault = result.page_fault();
            assert_eq!(fault.cr2(), COPYOUT_BAD_POINTER);
            assert_eq!(fault.error_code(), 0x2);
            assert_eq!(fault.rip(), COPYOUT_FAULT_RIP);
            assert_eq!(fault.cs(), 0x08);
            assert_eq!(fault.rflags(), 0x2 | X86_RFLAGS_RF);
            assert_eq!(result.final_cr2(), COPYOUT_BAD_POINTER);

            let frame = result.terminal_frame();
            assert_eq!(frame.rip(), COPYOUT_TERMINAL_RETURN_RIP);
            assert_eq!(frame.cs(), u64::from(PRIVILEGE_USER_CODE_SELECTOR));
            assert_eq!(frame.rflags(), 0x202);
            assert_eq!(frame.rsp(), PRIVILEGE_USER_STACK);
            assert_eq!(frame.ss(), u64::from(PRIVILEGE_USER_DATA_SELECTOR));

            assert_ne!(result.efer() & EFER_SYSCALL_ENABLE, 0);
            assert_eq!(result.star(), SYSCALL_STAR_VALUE);
            assert_eq!(result.lstar(), SYSCALL_LSTAR_VALUE);
            assert_eq!(result.sfmask(), SYSCALL_SFMASK_VALUE);

            assert_eq!(
                result.good_page_pte() & (X86_PAGE_USER | X86_PAGE_WRITE),
                X86_PAGE_USER | X86_PAGE_WRITE
            );
            assert_eq!(result.fault_handler_pte() & X86_PAGE_USER, 0);
            assert_eq!(result.fault_observation_pte() & X86_PAGE_USER, 0);
            assert_eq!(result.bad_pd_entry() & X86_PAGE_PRESENT, 0);

            assert_eq!(result.terminal_rsp(), PRIVILEGE_TSS_RSP0 - 40);
            assert_eq!(result.terminal_cs(), 0x08);
            assert_eq!(result.terminal_rflags() & 0x2, 0x2);
            assert_eq!(result.terminal_rflags() & 0x200, 0);
            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rip(), COPYOUT_TERMINAL_HLT_RIP);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping fault-safe copyout integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("fault-safe copyout guest execution failed unexpectedly: {error}"),
    }
}
