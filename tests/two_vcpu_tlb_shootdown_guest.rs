use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::long_mode::{
    LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS, LONG_MODE_EFER_REQUIRED_BITS,
    LONG_MODE_PML4_ADDR,
};
use mini_hypervisor::portio::two_vcpu_init_sipi_fixture::{
    AP_LONG_MODE_CODE_SELECTOR, AP_LONG_MODE_DATA_SELECTOR, AP_LONG_MODE_GDT,
    AP_LONG_MODE_GDT_LIMIT, AP_LONG_MODE_STACK,
};
use mini_hypervisor::portio::two_vcpu_tlb_shootdown_fixture::{
    run_two_vcpu_tlb_shootdown, AP_TLB_PROOF, BSP_TLB_PROOF, TLB_FINAL_PTE, TLB_PAGE_A,
    TLB_PAGE_A_VALUE, TLB_PAGE_B, TLB_PAGE_B_VALUE, TLB_SHOOTDOWN_IDT_LIMIT, TLB_SHOOTDOWN_VECTOR,
    TLB_TARGET_PTE, TLB_TARGET_VIRTUAL_PAGE,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::PortIoDirection;

const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;
const X86_RFLAGS_INTERRUPT_ENABLE: u64 = 1 << 9;
const KVM_MP_STATE_UNINITIALIZED: u32 = 1;
const PAGE_TABLE_ENTRY_FLAGS: u64 = 0x3;
const PAGE_TABLE_ENTRY_ACCESSED: u64 = 1 << 5;
const PAGE_TABLE_ENTRY_DIRTY: u64 = 1 << 6;

#[test]
fn sipi_started_ap_invalidates_shared_alias_after_remote_shootdown() {
    match run_two_vcpu_tlb_shootdown() {
        Ok(result) => {
            let state = result.state();
            assert_eq!(state.initial_ap_mp_state(), KVM_MP_STATE_UNINITIALIZED);
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
            assert_eq!(state.idt_limit(), TLB_SHOOTDOWN_IDT_LIMIT);
            assert_eq!(state.cr3(), LONG_MODE_PML4_ADDR.get());
            assert_eq!(
                state.cr0() & LONG_MODE_CR0_REQUIRED_BITS,
                LONG_MODE_CR0_REQUIRED_BITS
            );
            assert_eq!(
                state.cr4() & LONG_MODE_CR4_REQUIRED_BITS,
                LONG_MODE_CR4_REQUIRED_BITS
            );
            assert_eq!(
                state.efer() & LONG_MODE_EFER_REQUIRED_BITS,
                LONG_MODE_EFER_REQUIRED_BITS
            );

            assert_eq!(
                state.ready_rflags() & X86_RFLAGS_RESERVED_BIT,
                X86_RFLAGS_RESERVED_BIT
            );
            assert_eq!(state.ready_rflags() & X86_RFLAGS_INTERRUPT_ENABLE, 0);
            assert_eq!(
                state.completion_rflags() & (X86_RFLAGS_RESERVED_BIT | X86_RFLAGS_INTERRUPT_ENABLE),
                X86_RFLAGS_RESERVED_BIT | X86_RFLAGS_INTERRUPT_ENABLE
            );

            assert_eq!(TLB_TARGET_VIRTUAL_PAGE, 0x50_1000);
            assert_eq!(TLB_TARGET_PTE.get(), 0x4808);
            assert_eq!(TLB_SHOOTDOWN_VECTOR, 0x54);
            assert_eq!(TLB_PAGE_B.get() | PAGE_TABLE_ENTRY_FLAGS, 0x1_9003);
            assert_eq!(
                TLB_PAGE_B.get() | PAGE_TABLE_ENTRY_FLAGS | PAGE_TABLE_ENTRY_ACCESSED,
                TLB_FINAL_PTE
            );
            assert_eq!(result.final_pte(), TLB_FINAL_PTE);
            assert_eq!(
                result.final_pte() & PAGE_TABLE_ENTRY_ACCESSED,
                PAGE_TABLE_ENTRY_ACCESSED
            );
            assert_eq!(result.final_pte() & PAGE_TABLE_ENTRY_DIRTY, 0);
            assert_eq!(result.final_ack(), 0);
            assert_eq!(result.page_a(), TLB_PAGE_A_VALUE);
            assert_eq!(result.page_b(), TLB_PAGE_B_VALUE);
            assert_eq!(TLB_PAGE_A.get(), 0x1_8000);
            assert_eq!(TLB_PAGE_B.get(), 0x1_9000);

            assert_eq!(result.bsp_proof(), BSP_TLB_PROOF);
            assert_eq!(result.ap_proof(), AP_TLB_PROOF);
            assert_eq!(result.bsp_io_exits().len(), BSP_TLB_PROOF.len());
            for (exit, expected) in result
                .bsp_io_exits()
                .iter()
                .zip(BSP_TLB_PROOF.iter().copied())
            {
                assert_eq!(exit.direction(), PortIoDirection::Out);
                assert_eq!(exit.port(), DEBUG_PORT);
                assert_eq!(exit.size(), 1);
                assert_eq!(exit.count(), 1);
                assert_eq!(exit.output_data(), &[expected]);
            }

            assert_eq!(result.ap_io_exits().len(), AP_TLB_PROOF.len());
            for (exit, expected) in result
                .ap_io_exits()
                .iter()
                .zip(AP_TLB_PROOF.iter().copied())
            {
                assert_eq!(exit.direction(), PortIoDirection::Out);
                assert_eq!(exit.port(), DEBUG_PORT);
                assert_eq!(exit.size(), 1);
                assert_eq!(exit.count(), 1);
                assert_eq!(exit.output_data(), &[expected]);
            }
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping TLB-shootdown integration assertion: /dev/kvm is unavailable");
        }
        Err(error) => panic!("two-vCPU TLB-shootdown execution failed unexpectedly: {error}"),
    }
}
