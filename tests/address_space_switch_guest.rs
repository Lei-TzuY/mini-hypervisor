use mini_hypervisor::address_space::{
    run_address_space_switch_guest, ADDRESS_SPACE_A_CR3, ADDRESS_SPACE_A_DATA_VALUE,
    ADDRESS_SPACE_B_PML4_ADDR, ADDRESS_SPACE_B_USER_CODE_BACKING,
    ADDRESS_SPACE_B_USER_DATA_BACKING, ADDRESS_SPACE_B_USER_STACK_BACKING, ADDRESS_SPACE_PROOF,
    ADDRESS_SPACE_TERMINAL_RIP,
};
use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::long_mode::LONG_MODE_PAGE_SIZE;
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::privilege::{
    PRIVILEGE_RETURN_HANDLER, PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_STACK,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITABLE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_PAGE_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

#[test]
fn switches_between_isolated_ring3_address_spaces_and_back() {
    match run_address_space_switch_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.proof(), ADDRESS_SPACE_PROOF);
            assert_eq!(result.io_exits().len(), ADDRESS_SPACE_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(ADDRESS_SPACE_PROOF.iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.size(), 1);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            assert_eq!(
                result.cr3_observations(),
                [
                    ADDRESS_SPACE_A_CR3.get(),
                    ADDRESS_SPACE_B_PML4_ADDR.get(),
                    ADDRESS_SPACE_A_CR3.get(),
                ]
            );
            assert_eq!(result.final_cr3(), ADDRESS_SPACE_A_CR3.get());
            assert_eq!(result.first_data(), ADDRESS_SPACE_A_DATA_VALUE);
            assert_eq!(result.second_data(), b'B');

            validate_pte(result.first_code_pte(), PRIVILEGE_USER_ENTRY.get(), true);
            validate_pte(
                result.second_code_pte(),
                ADDRESS_SPACE_B_USER_CODE_BACKING.get(),
                true,
            );
            validate_pte(result.first_data_pte(), 0xa000, true);
            validate_pte(
                result.second_data_pte(),
                ADDRESS_SPACE_B_USER_DATA_BACKING.get(),
                true,
            );
            validate_pte(
                result.first_stack_pte(),
                (PRIVILEGE_USER_STACK - 1) & !(LONG_MODE_PAGE_SIZE - 1),
                true,
            );
            validate_pte(
                result.second_stack_pte(),
                ADDRESS_SPACE_B_USER_STACK_BACKING.get(),
                true,
            );
            validate_pte(
                result.first_kernel_pte(),
                PRIVILEGE_RETURN_HANDLER.get(),
                false,
            );
            validate_pte(
                result.second_kernel_pte(),
                PRIVILEGE_RETURN_HANDLER.get(),
                false,
            );

            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rip(), ADDRESS_SPACE_TERMINAL_RIP);
            assert_eq!(result.report().rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping address-space switch integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("address-space switch guest execution failed unexpectedly: {error}"),
    }
}

fn validate_pte(pte: u64, physical: u64, user: bool) {
    assert_eq!(pte & X86_PAGE_ADDRESS_MASK, physical);
    assert_eq!(pte & X86_PAGE_PRESENT, X86_PAGE_PRESENT);
    assert_eq!(pte & X86_PAGE_WRITABLE, X86_PAGE_WRITABLE);
    assert_eq!(pte & X86_PAGE_USER != 0, user);
}
