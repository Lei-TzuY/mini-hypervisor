use mini_hypervisor::error::{Error, HostEnvironmentError, VmExitError};
use mini_hypervisor::execution::run_vcpu_until_stopped;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::loader::FlatGuestImage;
use mini_hypervisor::memory::{GuestMemory, GuestPhysAddr};
use mini_hypervisor::portio::PortIoBus;
use mini_hypervisor::vcpu::VcpuId;

const KVM_EXIT_IO: u32 = 2;
const RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const RAM_SIZE: u64 = 2 * 1024 * 1024;
const ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const GUEST_BYTES: [u8; 5] = [0xb0, b'K', 0xe6, 0xe9, 0xf4];

fn backend_or_skip() -> Option<KvmBackend> {
    match KvmBackend::open() {
        Ok(backend) => Some(backend),
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping KVM integration assertion: /dev/kvm is unavailable to this runner");
            None
        }
        Err(error) => panic!("KVM backend initialization failed unexpectedly: {error}"),
    }
}

#[test]
fn budget_exhaustion_preserves_completed_exit_trace_without_extra_kvm_run_when_kvm_is_available() {
    let Some(backend) = backend_or_skip() else {
        return;
    };
    let image = FlatGuestImage::new(ENTRY, ENTRY, &GUEST_BYTES)
        .expect("debug-port guest image should be valid");
    let mut vm = backend.create_vm().expect("VM creation should succeed");
    let mut memory = GuestMemory::new(RAM_BASE, RAM_SIZE).expect("guest RAM should map");
    image.load(&mut memory).expect("guest image should load");
    vm.register_guest_memory(memory)
        .expect("guest RAM registration should succeed");
    let mut vcpu = vm
        .create_vcpu(VcpuId::BOOT)
        .expect("vCPU creation should succeed");
    vcpu.initialize_real_mode(image.entry())
        .expect("real-mode initialization should succeed");
    let mut port_io = PortIoBus::with_debug_port();

    let error = run_vcpu_until_stopped(&mut vcpu, &mut port_io, 1)
        .expect_err("one-exit budget should stop before the pending debug-port I/O is completed");

    match error {
        Error::VmExit(VmExitError::ExitBudgetExhausted {
            vcpu_id,
            budget,
            completed,
            last_exit_reason,
            exit_reasons,
        }) => {
            assert_eq!(vcpu_id, 0);
            assert_eq!(budget, 1);
            assert_eq!(completed, 1);
            assert_eq!(last_exit_reason, Some(KVM_EXIT_IO));
            assert_eq!(exit_reasons, [KVM_EXIT_IO]);
            assert_eq!(completed as usize, exit_reasons.len());
            assert_eq!(last_exit_reason, exit_reasons.last().copied());
        }
        other => panic!("unexpected execution error: {other}"),
    }
}
