use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::{LONG_MODE_INTERRUPT_IDT_ADDR, X86_RFLAGS_INTERRUPT_ENABLE};
use mini_hypervisor::long_mode::{
    LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS, LONG_MODE_EFER_REQUIRED_BITS,
    LONG_MODE_PML4_ADDR,
};
use mini_hypervisor::portio::two_vcpu_init_sipi_fixture::{
    run_two_vcpu_ap_long_mode_ipi, AP_LONG_MODE_CODE_SELECTOR, AP_LONG_MODE_DATA_SELECTOR,
    AP_LONG_MODE_GDT, AP_LONG_MODE_GDT_LIMIT, AP_LONG_MODE_IPI_BSP_PROOF,
    AP_LONG_MODE_IPI_IDT_LIMIT, AP_LONG_MODE_IPI_PROOF, AP_LONG_MODE_IPI_VECTOR,
    AP_LONG_MODE_STACK,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::PortIoDirection;

#[test]
fn sipi_started_long_mode_ap_handles_guest_originated_ipi() {
    match run_two_vcpu_ap_long_mode_ipi() {
        Ok(result) => {
            assert_eq!(result.initial_mp_state(), 1);
            assert_eq!(result.startup_mp_state(), 0);
            assert_eq!(result.startup_rip(), 0);
            assert_eq!(result.startup_cs_selector(), 0x0800);
            assert_eq!(result.startup_cs_base(), 0x8000);
            assert_eq!(result.startup_cr0() & 1, 0);
            assert_eq!(result.final_mp_state(), 0);
            assert_eq!(result.shared_marker(), b'K');
            assert_eq!(result.first_proof(), AP_LONG_MODE_IPI_BSP_PROOF);
            assert_eq!(result.second_proof(), AP_LONG_MODE_IPI_PROOF);
            assert_eq!(
                result.first_io_exits().len(),
                AP_LONG_MODE_IPI_BSP_PROOF.len()
            );
            assert_eq!(result.second_io_exits().len(), AP_LONG_MODE_IPI_PROOF.len());

            for (io, expected) in result
                .first_io_exits()
                .iter()
                .zip(AP_LONG_MODE_IPI_BSP_PROOF.iter().copied())
                .chain(
                    result
                        .second_io_exits()
                        .iter()
                        .zip(AP_LONG_MODE_IPI_PROOF.iter().copied()),
                )
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            let long_mode = result.long_mode_state();
            assert_eq!(long_mode.rsp(), AP_LONG_MODE_STACK);
            assert_eq!(long_mode.cs_selector(), AP_LONG_MODE_CODE_SELECTOR);
            assert_eq!(long_mode.cs_long(), 1);
            assert_eq!(long_mode.ss_selector(), AP_LONG_MODE_DATA_SELECTOR);
            assert_eq!(long_mode.gdt_base(), AP_LONG_MODE_GDT.get());
            assert_eq!(long_mode.gdt_limit(), AP_LONG_MODE_GDT_LIMIT);
            assert_eq!(
                long_mode.cr0() & LONG_MODE_CR0_REQUIRED_BITS,
                LONG_MODE_CR0_REQUIRED_BITS
            );
            assert_eq!(long_mode.cr3(), LONG_MODE_PML4_ADDR.get());
            assert_eq!(
                long_mode.cr4() & LONG_MODE_CR4_REQUIRED_BITS,
                LONG_MODE_CR4_REQUIRED_BITS
            );
            assert_eq!(
                long_mode.efer() & LONG_MODE_EFER_REQUIRED_BITS,
                LONG_MODE_EFER_REQUIRED_BITS
            );

            let interrupt = result.interrupt_state();
            assert_eq!(AP_LONG_MODE_IPI_VECTOR, 0x52);
            assert_eq!(interrupt.idt_base(), LONG_MODE_INTERRUPT_IDT_ADDR.get());
            assert_eq!(interrupt.idt_limit(), AP_LONG_MODE_IPI_IDT_LIMIT);
            assert_eq!(interrupt.ready_rflags() & 0x2, 0x2);
            assert_eq!(interrupt.ready_rflags() & X86_RFLAGS_INTERRUPT_ENABLE, 0);
            assert_eq!(result.ap_completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.ap_completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping AP long-mode IPI integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("AP long-mode IPI guest execution failed unexpectedly: {error}"),
    }
}
