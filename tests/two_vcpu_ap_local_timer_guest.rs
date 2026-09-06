use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::long_mode::{
    LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS, LONG_MODE_EFER_REQUIRED_BITS,
    LONG_MODE_PML4_ADDR,
};
use mini_hypervisor::portio::two_vcpu_ap_local_timer_fixture::{
    run_two_vcpu_ap_local_timer, AP_LOCAL_TIMER_IDT_LIMIT, AP_LOCAL_TIMER_VECTOR,
    AP_TIMER_PROOF, BSP_TIMER_PROOF,
};
use mini_hypervisor::portio::two_vcpu_init_sipi_fixture::{
    AP_LONG_MODE_CODE_SELECTOR, AP_LONG_MODE_DATA_SELECTOR, AP_LONG_MODE_GDT,
    AP_LONG_MODE_GDT_LIMIT, AP_LONG_MODE_STACK,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::PortIoDirection;

const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;
const X86_RFLAGS_INTERRUPT_ENABLE: u64 = 1 << 9;
const KVM_MP_STATE_RUNNABLE: u32 = 0;
const KVM_MP_STATE_UNINITIALIZED: u32 = 1;

#[test]
fn sipi_started_ap_owns_one_shot_local_lapic_timer() {
    match run_two_vcpu_ap_local_timer() {
        Ok(result) => {
            assert_eq!(result.initial_ap_mp_state(), KVM_MP_STATE_UNINITIALIZED);
            assert!(!result.watchdog_fired());
            assert_eq!(result.shared_marker(), b'K');
            assert_eq!(result.bsp_proof(), BSP_TIMER_PROOF);
            assert_eq!(result.ap_proof(), AP_TIMER_PROOF);

            let state = result.ap_state();
            assert_eq!(state.startup_mp_state(), KVM_MP_STATE_RUNNABLE);
            assert_eq!(state.startup_rip(), 0);
            assert_eq!(state.startup_cs_selector(), 0x0800);
            assert_eq!(state.startup_cs_base(), 0x8000);
            assert_eq!(state.rsp(), AP_LONG_MODE_STACK);
            assert_eq!(state.cs_selector(), AP_LONG_MODE_CODE_SELECTOR);
            assert_eq!(state.cs_long(), 1);
            assert_eq!(state.ss_selector(), AP_LONG_MODE_DATA_SELECTOR);
            assert_eq!(state.gdt_base(), AP_LONG_MODE_GDT.get());
            assert_eq!(state.gdt_limit(), AP_LONG_MODE_GDT_LIMIT);
            assert_eq!(state.idt_base(), 0x6000);
            assert_eq!(state.idt_limit(), AP_LOCAL_TIMER_IDT_LIMIT);
            assert_eq!(state.cr3(), LONG_MODE_PML4_ADDR.get());
            assert_eq!(state.cr0() & LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR0_REQUIRED_BITS);
            assert_eq!(state.cr4() & LONG_MODE_CR4_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS);
            assert_eq!(state.efer() & LONG_MODE_EFER_REQUIRED_BITS, LONG_MODE_EFER_REQUIRED_BITS);

            for rflags in [state.ready_rflags(), state.armed_rflags()] {
                assert_eq!(rflags & X86_RFLAGS_RESERVED_BIT, X86_RFLAGS_RESERVED_BIT);
                assert_eq!(rflags & X86_RFLAGS_INTERRUPT_ENABLE, 0);
            }
            assert_eq!(
                state.completion_rflags()
                    & (X86_RFLAGS_RESERVED_BIT | X86_RFLAGS_INTERRUPT_ENABLE),
                X86_RFLAGS_RESERVED_BIT | X86_RFLAGS_INTERRUPT_ENABLE
            );

            assert_eq!(result.bsp_io_exits().len(), BSP_TIMER_PROOF.len());
            for (exit, expected) in result
                .bsp_io_exits()
                .iter()
                .zip(BSP_TIMER_PROOF.iter().copied())
            {
                assert_eq!(exit.direction(), PortIoDirection::Out);
                assert_eq!(exit.port(), DEBUG_PORT);
                assert_eq!(exit.size(), 1);
                assert_eq!(exit.count(), 1);
                assert_eq!(exit.output_data(), &[expected]);
            }

            assert_eq!(result.ap_io_exits().len(), AP_TIMER_PROOF.len());
            for (exit, expected) in result
                .ap_io_exits()
                .iter()
                .zip(AP_TIMER_PROOF.iter().copied())
            {
                assert_eq!(exit.direction(), PortIoDirection::Out);
                assert_eq!(exit.port(), DEBUG_PORT);
                assert_eq!(exit.size(), 1);
                assert_eq!(exit.count(), 1);
                assert_eq!(exit.output_data(), &[expected]);
            }

            assert_eq!(AP_LOCAL_TIMER_VECTOR, 0x53);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping AP local timer integration assertion: /dev/kvm is unavailable");
        }
        Err(error) => panic!("AP local timer execution failed unexpectedly: {error}"),
    }
}
