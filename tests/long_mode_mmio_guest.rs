use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::mmio::long_mode::{
    run_long_mode_mmio_guest, LONG_MODE_MMIO_DEVICE_GPA, LONG_MODE_MMIO_PROOF,
    LONG_MODE_MMIO_TERMINAL_RIP, LONG_MODE_MMIO_WRITE_VALUE,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{MmioDirection, PortIoDirection, VcpuExit, VcpuId};

#[test]
fn long_mode_guest_translates_virtual_device_access_to_mmio_and_continues() {
    match run_long_mode_mmio_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.writes(), &[LONG_MODE_MMIO_WRITE_VALUE]);
            assert_eq!(result.proof(), LONG_MODE_MMIO_PROOF);

            assert_eq!(result.mmio_exits().len(), 2);
            let write = &result.mmio_exits()[0];
            assert_eq!(write.address(), LONG_MODE_MMIO_DEVICE_GPA);
            assert_eq!(write.direction(), MmioDirection::Write);
            assert_eq!(write.length(), 1);
            assert_eq!(write.write_data(), &[LONG_MODE_MMIO_WRITE_VALUE]);

            let read = &result.mmio_exits()[1];
            assert_eq!(read.address(), LONG_MODE_MMIO_DEVICE_GPA);
            assert_eq!(read.direction(), MmioDirection::Read);
            assert_eq!(read.length(), 1);
            assert!(read.write_data().is_empty());

            assert_eq!(result.io_exits().len(), LONG_MODE_MMIO_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(LONG_MODE_MMIO_PROOF.iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            let report = result.report();
            assert_eq!(report.vcpu_id(), VcpuId::BOOT);
            assert_eq!(report.exit(), VcpuExit::Hlt);
            assert_eq!(report.rip(), LONG_MODE_MMIO_TERMINAL_RIP);
            assert_eq!(report.rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping long-mode virtual-MMIO integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("long-mode virtual-MMIO guest execution failed unexpectedly: {error}"),
    }
}
