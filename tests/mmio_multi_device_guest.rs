use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::mmio::long_mode::{LONG_MODE_MMIO_DEVICE_GPA, LONG_MODE_MMIO_VIRTUAL_PAGE};
use mini_hypervisor::mmio::multi_device::{
    run_multi_device_mmio_guest, MULTI_DEVICE_FIRST_READ_VALUE, MULTI_DEVICE_FIRST_WRITE_VALUE,
    MULTI_DEVICE_PROOF, MULTI_DEVICE_SECOND_GPA, MULTI_DEVICE_SECOND_READ_VALUE,
    MULTI_DEVICE_SECOND_VIRTUAL_PAGE, MULTI_DEVICE_SECOND_WRITE_VALUE, MULTI_DEVICE_TERMINAL_RIP,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{MmioDirection, PortIoDirection, VcpuExit};

#[test]
fn two_virtual_mmio_devices_dispatch_independently_and_halt() {
    match run_multi_device_mmio_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(LONG_MODE_MMIO_VIRTUAL_PAGE, 0x50_0000);
            assert_eq!(MULTI_DEVICE_SECOND_VIRTUAL_PAGE, 0x50_1000);
            assert_eq!(LONG_MODE_MMIO_DEVICE_GPA, 0x1000_0000);
            assert_eq!(MULTI_DEVICE_SECOND_GPA, 0x1000_1000);

            assert_eq!(result.first_writes(), &[MULTI_DEVICE_FIRST_WRITE_VALUE]);
            assert_eq!(result.second_writes(), &[MULTI_DEVICE_SECOND_WRITE_VALUE]);
            assert_eq!(result.proof(), MULTI_DEVICE_PROOF);

            let exits = result.mmio_exits();
            assert_eq!(exits.len(), 5);
            let expected = [
                (
                    LONG_MODE_MMIO_DEVICE_GPA,
                    MmioDirection::Write,
                    &[MULTI_DEVICE_FIRST_WRITE_VALUE][..],
                ),
                (LONG_MODE_MMIO_DEVICE_GPA, MmioDirection::Read, &[][..]),
                (
                    MULTI_DEVICE_SECOND_GPA,
                    MmioDirection::Write,
                    &[MULTI_DEVICE_SECOND_WRITE_VALUE][..],
                ),
                (MULTI_DEVICE_SECOND_GPA, MmioDirection::Read, &[][..]),
                (LONG_MODE_MMIO_DEVICE_GPA, MmioDirection::Read, &[][..]),
            ];
            for (exit, (address, direction, write_data)) in exits.iter().zip(expected) {
                assert_eq!(exit.address(), address);
                assert_eq!(exit.direction(), direction);
                assert_eq!(exit.length(), 1);
                assert_eq!(exit.write_data(), write_data);
            }

            assert_eq!(result.io_exits().len(), MULTI_DEVICE_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(MULTI_DEVICE_PROOF.iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            assert_eq!(MULTI_DEVICE_FIRST_READ_VALUE, b'A');
            assert_eq!(MULTI_DEVICE_SECOND_READ_VALUE, b'B');
            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rip(), MULTI_DEVICE_TERMINAL_RIP);
            assert_eq!(result.report().rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping multi-device MMIO integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("multi-device MMIO guest execution failed unexpectedly: {error}"),
    }
}
