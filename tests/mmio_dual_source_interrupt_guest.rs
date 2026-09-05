use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::mmio::dual_source_interrupt::{
    run_dual_source_mmio_interrupt_guest, DUAL_SOURCE_FIRST_GSI, DUAL_SOURCE_FIRST_VECTOR,
    DUAL_SOURCE_PROOF, DUAL_SOURCE_SECOND_GSI, DUAL_SOURCE_SECOND_HANDLER,
    DUAL_SOURCE_SECOND_VECTOR,
};
use mini_hypervisor::mmio::level_interrupt::{
    MMIO_LEVEL_INTERRUPT_ACK_VALUE, MMIO_LEVEL_INTERRUPT_COMMAND_VALUE,
};
use mini_hypervisor::mmio::long_mode::LONG_MODE_MMIO_DEVICE_GPA;
use mini_hypervisor::mmio::multi_device::MULTI_DEVICE_SECOND_GPA;
use mini_hypervisor::mmio::{LEVEL_INTERRUPT_ACK_OFFSET, LEVEL_INTERRUPT_STATUS_OFFSET};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{MmioDirection, PortIoDirection};

const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_DELIVERY_MODE_EXTINT: u32 = 0x700;

#[test]
fn two_level_mmio_sources_route_to_distinct_pic_vectors() {
    match run_dual_source_mmio_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            let routes = result.routes();
            assert_eq!(routes.len(), 2);
            assert_eq!(routes[0].device_address(), LONG_MODE_MMIO_DEVICE_GPA);
            assert_eq!(routes[0].gsi(), DUAL_SOURCE_FIRST_GSI);
            assert_eq!(routes[0].vector(), DUAL_SOURCE_FIRST_VECTOR);
            assert_eq!(routes[1].device_address(), MULTI_DEVICE_SECOND_GPA);
            assert_eq!(routes[1].gsi(), DUAL_SOURCE_SECOND_GSI);
            assert_eq!(routes[1].vector(), DUAL_SOURCE_SECOND_VECTOR);
            assert_eq!(DUAL_SOURCE_FIRST_GSI, 0);
            assert_eq!(DUAL_SOURCE_SECOND_GSI, 1);
            assert_eq!(DUAL_SOURCE_FIRST_VECTOR, 0x40);
            assert_eq!(DUAL_SOURCE_SECOND_VECTOR, 0x41);
            assert_eq!(DUAL_SOURCE_SECOND_HANDLER.get(), 0x1_2000);

            assert_eq!(result.assert_event_count(), 2);
            assert_eq!(result.deassert_event_count(), 2);
            assert_eq!(
                result.first_writes(),
                &[
                    MMIO_LEVEL_INTERRUPT_COMMAND_VALUE,
                    MMIO_LEVEL_INTERRUPT_ACK_VALUE
                ]
            );
            assert_eq!(
                result.second_writes(),
                &[
                    MMIO_LEVEL_INTERRUPT_COMMAND_VALUE,
                    MMIO_LEVEL_INTERRUPT_ACK_VALUE
                ]
            );
            assert_eq!(result.proof(), DUAL_SOURCE_PROOF);

            let exits = result.mmio_exits();
            assert_eq!(exits.len(), 6);
            let expected = [
                (
                    LONG_MODE_MMIO_DEVICE_GPA,
                    MmioDirection::Write,
                    &[MMIO_LEVEL_INTERRUPT_COMMAND_VALUE][..],
                ),
                (
                    LONG_MODE_MMIO_DEVICE_GPA + LEVEL_INTERRUPT_STATUS_OFFSET,
                    MmioDirection::Read,
                    &[][..],
                ),
                (
                    LONG_MODE_MMIO_DEVICE_GPA + LEVEL_INTERRUPT_ACK_OFFSET,
                    MmioDirection::Write,
                    &[MMIO_LEVEL_INTERRUPT_ACK_VALUE][..],
                ),
                (
                    MULTI_DEVICE_SECOND_GPA,
                    MmioDirection::Write,
                    &[MMIO_LEVEL_INTERRUPT_COMMAND_VALUE][..],
                ),
                (
                    MULTI_DEVICE_SECOND_GPA + LEVEL_INTERRUPT_STATUS_OFFSET,
                    MmioDirection::Read,
                    &[][..],
                ),
                (
                    MULTI_DEVICE_SECOND_GPA + LEVEL_INTERRUPT_ACK_OFFSET,
                    MmioDirection::Write,
                    &[MMIO_LEVEL_INTERRUPT_ACK_VALUE][..],
                ),
            ];
            for (exit, (address, direction, write_data)) in exits.iter().zip(expected) {
                assert_eq!(exit.address(), address);
                assert_eq!(exit.direction(), direction);
                assert_eq!(exit.length(), 1);
                assert_eq!(exit.write_data(), write_data);
            }

            assert_eq!(result.io_exits().len(), DUAL_SOURCE_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(DUAL_SOURCE_PROOF.iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            assert_eq!(
                result.lapic_spiv() & APIC_SPIV_SOFTWARE_ENABLE,
                APIC_SPIV_SOFTWARE_ENABLE
            );
            assert_eq!(
                result.lapic_lint0() & APIC_LVT_DELIVERY_MODE_MASK,
                APIC_LVT_DELIVERY_MODE_EXTINT
            );
            assert_eq!(result.lapic_lint0() & APIC_LVT_MASKED, 0);

            for rflags in result
                .armed_rflags()
                .into_iter()
                .chain(std::iter::once(result.completion_rflags()))
            {
                assert_eq!(rflags & 0x2, 0x2);
                assert_eq!(
                    rflags & X86_RFLAGS_INTERRUPT_ENABLE,
                    X86_RFLAGS_INTERRUPT_ENABLE
                );
            }
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping dual-source MMIO interrupt integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => {
            panic!("dual-source MMIO interrupt guest execution failed unexpectedly: {error}")
        }
    }
}
