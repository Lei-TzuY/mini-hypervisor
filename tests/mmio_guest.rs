use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::mmio_fixture::{
    mmio_device_address, run_mmio_guest, MMIO_GUEST_PROOF, MMIO_GUEST_TERMINAL_RIP,
    MMIO_GUEST_WRITE_VALUE,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{MmioDirection, PortIoDirection, VcpuExit, VcpuId};

#[test]
fn deterministic_guest_services_mmio_write_and_read_then_halts() {
    match run_mmio_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.writes(), &[MMIO_GUEST_WRITE_VALUE]);
            assert_eq!(result.proof(), MMIO_GUEST_PROOF);

            assert_eq!(result.mmio_exits().len(), 2);
            let write = &result.mmio_exits()[0];
            assert_eq!(write.address(), mmio_device_address());
            assert_eq!(write.direction(), MmioDirection::Write);
            assert_eq!(write.length(), 1);
            assert_eq!(write.write_data(), &[MMIO_GUEST_WRITE_VALUE]);

            let read = &result.mmio_exits()[1];
            assert_eq!(read.address(), mmio_device_address());
            assert_eq!(read.direction(), MmioDirection::Read);
            assert_eq!(read.length(), 1);
            assert!(read.write_data().is_empty());

            assert_eq!(result.io_exits().len(), MMIO_GUEST_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(MMIO_GUEST_PROOF.iter().copied())
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
            assert_eq!(report.rip(), MMIO_GUEST_TERMINAL_RIP);
            assert_eq!(report.rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping MMIO execution integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("MMIO guest execution failed unexpectedly: {error}"),
    }
}
