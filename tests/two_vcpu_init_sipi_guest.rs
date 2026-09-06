use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::two_vcpu_init_sipi_fixture::{
    run_two_vcpu_init_sipi, FIRST_PROOF, SECOND_PROOF, SIPI_VECTOR,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::PortIoDirection;

#[test]
fn guest_init_sipi_starts_uninitialized_ap_at_real_mode_trampoline() {
    match run_two_vcpu_init_sipi() {
        Ok(result) => {
            assert_eq!(SIPI_VECTOR, 0x08);
            assert_eq!(u64::from(SIPI_VECTOR) << 12, 0x8000);
            assert_eq!(result.initial_mp_state(), 1);
            assert_eq!(result.final_mp_state(), 0);
            assert_eq!(result.shared_marker(), b'K');
            assert_eq!(result.first_proof(), FIRST_PROOF);
            assert_eq!(result.second_proof(), SECOND_PROOF);
            assert_eq!(result.first_io_exits().len(), FIRST_PROOF.len());
            assert_eq!(result.second_io_exits().len(), SECOND_PROOF.len());

            for (io, expected) in result
                .first_io_exits()
                .iter()
                .zip(FIRST_PROOF.iter().copied())
                .chain(
                    result
                        .second_io_exits()
                        .iter()
                        .zip(SECOND_PROOF.iter().copied()),
                )
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            assert_eq!(result.ap_completion_rflags() & 0x2, 0x2);
            assert_eq!(result.ap_completion_rflags() & 0x200, 0);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping INIT/SIPI integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("INIT/SIPI guest execution failed unexpectedly: {error}"),
    }
}
