use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::long_mode::{
    LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS, LONG_MODE_EFER_REQUIRED_BITS,
    LONG_MODE_PML4_ADDR,
};
use mini_hypervisor::portio::two_vcpu_init_sipi_fixture::{
    run_two_vcpu_ap_long_mode, AP_LONG_MODE_CODE_SELECTOR, AP_LONG_MODE_DATA_SELECTOR,
    AP_LONG_MODE_GDT, AP_LONG_MODE_GDT_LIMIT, AP_LONG_MODE_PROOF, AP_LONG_MODE_STACK,
    FIRST_PROOF,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::PortIoDirection;

#[test]
fn sipi_started_ap_enters_long_mode_from_guest_trampoline() {
    match run_two_vcpu_ap_long_mode() {
        Ok(result) => {
            assert_eq!(result.initial_mp_state(), 1);
            assert_eq!(result.startup_mp_state(), 0);
            assert_eq!(result.startup_rip(), 0);
            assert_eq!(result.startup_cs_selector(), 0x0800);
            assert_eq!(result.startup_cs_base(), 0x8000);
            assert_eq!(result.startup_cr0() & 1, 0);
            assert_eq!(result.final_mp_state(), 0);
            assert_eq!(result.shared_marker(), b'K');
            assert_eq!(result.first_proof(), FIRST_PROOF);
            assert_eq!(result.second_proof(), AP_LONG_MODE_PROOF);
            assert_eq!(result.first_io_exits().len(), FIRST_PROOF.len());
            assert_eq!(result.second_io_exits().len(), AP_LONG_MODE_PROOF.len());

            for (io, expected) in result
                .first_io_exits()
                .iter()
                .zip(FIRST_PROOF.iter().copied())
                .chain(
                    result
                        .second_io_exits()
                        .iter()
                        .zip(AP_LONG_MODE_PROOF.iter().copied()),
                )
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            let state = result.long_mode_state();
            assert_eq!(state.rsp(), AP_LONG_MODE_STACK);
            assert_eq!(state.cs_selector(), AP_LONG_MODE_CODE_SELECTOR);
            assert_eq!(state.cs_long(), 1);
            assert_eq!(state.ss_selector(), AP_LONG_MODE_DATA_SELECTOR);
            assert_eq!(state.gdt_base(), AP_LONG_MODE_GDT.get());
            assert_eq!(state.gdt_limit(), AP_LONG_MODE_GDT_LIMIT);
            assert_eq!(state.cr0() & LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR0_REQUIRED_BITS);
            assert_eq!(state.cr3(), LONG_MODE_PML4_ADDR.get());
            assert_eq!(state.cr4() & LONG_MODE_CR4_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS);
            assert_eq!(state.efer() & LONG_MODE_EFER_REQUIRED_BITS, LONG_MODE_EFER_REQUIRED_BITS);
            assert_eq!(result.ap_completion_rflags() & 0x2, 0x2);
            assert_eq!(result.ap_completion_rflags() & 0x200, 0);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping AP long-mode integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("AP long-mode guest execution failed unexpectedly: {error}"),
    }
}
