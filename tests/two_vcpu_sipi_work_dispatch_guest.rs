use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::LONG_MODE_INTERRUPT_IDT_ADDR;
use mini_hypervisor::long_mode::{
    LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS, LONG_MODE_EFER_REQUIRED_BITS,
    LONG_MODE_PML4_ADDR,
};
use mini_hypervisor::portio::two_vcpu_init_sipi_fixture::{
    AP_LONG_MODE_CODE_SELECTOR, AP_LONG_MODE_DATA_SELECTOR, AP_LONG_MODE_GDT,
    AP_LONG_MODE_GDT_LIMIT, AP_LONG_MODE_IPI_IDT_LIMIT, AP_LONG_MODE_STACK,
};
use mini_hypervisor::portio::two_vcpu_sipi_work_dispatch_fixture::{
    run_sipi_ipi_work_dispatch, AP_COMPOSED_PROOF, BSP_COMPOSED_PROOF,
};
use mini_hypervisor::portio::two_vcpu_work_dispatch_fixture::{WORK_PAYLOAD, WORK_RESULT};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::PortIoDirection;

const RFLAGS_BIT1: u64 = 1 << 1;
const RFLAGS_IF: u64 = 1 << 9;

#[test]
fn sipi_started_ap_receives_ipi_and_completes_locked_mailbox_work() {
    match run_sipi_ipi_work_dispatch() {
        Ok(result) => {
            assert_eq!(result.bsp_proof(), BSP_COMPOSED_PROOF);
            assert_eq!(result.ap_proof(), AP_COMPOSED_PROOF);
            assert_eq!(result.bsp_io_exits().len(), BSP_COMPOSED_PROOF.len());
            assert_eq!(result.ap_io_exits().len(), AP_COMPOSED_PROOF.len());
            for (io, expected) in result
                .bsp_io_exits()
                .iter()
                .zip(BSP_COMPOSED_PROOF.iter().copied())
                .chain(
                    result
                        .ap_io_exits()
                        .iter()
                        .zip(AP_COMPOSED_PROOF.iter().copied()),
                )
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            let mailbox = result.mailbox();
            assert_eq!(mailbox.payload(), WORK_PAYLOAD);
            assert_eq!(mailbox.command(), 0);
            assert_eq!(mailbox.result(), WORK_RESULT);
            assert_eq!(mailbox.ack(), 0);

            assert_eq!(result.initial_ap_mp_state(), 1);
            let ap = result.ap_state();
            assert_eq!(ap.startup_mp_state(), 0);
            assert_eq!(ap.startup_rip(), 0);
            assert_eq!(ap.startup_cs_selector(), 0x0800);
            assert_eq!(ap.startup_cs_base(), 0x8000);
            assert_eq!(ap.ready_rflags() & RFLAGS_BIT1, RFLAGS_BIT1);
            assert_eq!(ap.ready_rflags() & RFLAGS_IF, 0);
            assert_eq!(ap.completion_rflags() & RFLAGS_BIT1, RFLAGS_BIT1);
            assert_eq!(ap.completion_rflags() & RFLAGS_IF, RFLAGS_IF);
            assert_eq!(ap.rsp(), AP_LONG_MODE_STACK);
            assert_eq!(ap.cs_selector(), AP_LONG_MODE_CODE_SELECTOR);
            assert_eq!(ap.cs_long(), 1);
            assert_eq!(ap.ss_selector(), AP_LONG_MODE_DATA_SELECTOR);
            assert_eq!(ap.gdt_base(), AP_LONG_MODE_GDT.get());
            assert_eq!(ap.gdt_limit(), AP_LONG_MODE_GDT_LIMIT);
            assert_eq!(ap.idt_base(), LONG_MODE_INTERRUPT_IDT_ADDR.get());
            assert_eq!(ap.idt_limit(), AP_LONG_MODE_IPI_IDT_LIMIT);
            assert_eq!(ap.cr3(), LONG_MODE_PML4_ADDR.get());
            assert_eq!(
                ap.cr0() & LONG_MODE_CR0_REQUIRED_BITS,
                LONG_MODE_CR0_REQUIRED_BITS
            );
            assert_eq!(
                ap.cr4() & LONG_MODE_CR4_REQUIRED_BITS,
                LONG_MODE_CR4_REQUIRED_BITS
            );
            assert_eq!(
                ap.efer() & LONG_MODE_EFER_REQUIRED_BITS,
                LONG_MODE_EFER_REQUIRED_BITS
            );

            let bsp_completion = result.bsp_completion_rflags();
            assert_eq!(bsp_completion & RFLAGS_BIT1, RFLAGS_BIT1);
            assert_eq!(bsp_completion & RFLAGS_IF, 0);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping SIPI/IPI work-dispatch integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("SIPI/IPI work-dispatch guest execution failed unexpectedly: {error}"),
    }
}
