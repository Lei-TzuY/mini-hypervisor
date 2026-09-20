use crate::kvm::sys::{
    default_two_host_registration_pair, TWO_HOST_REGISTRATION_FIRST_BAR,
    TWO_HOST_REGISTRATION_SECOND_BAR,
};
use crate::loader::FlatGuestImage;
use crate::long_mode::{LongModeBootLayout, LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE};
use crate::memory::GuestMemory;
use crate::portio::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::state_snapshot::{
    TWO_VCPU_CHECKPOINT_FIRST_ENTRY, TWO_VCPU_CHECKPOINT_FIRST_ID,
    TWO_VCPU_CHECKPOINT_FIRST_MARKER, TWO_VCPU_CHECKPOINT_FIRST_PROOF,
    TWO_VCPU_CHECKPOINT_FIRST_STACK, TWO_VCPU_CHECKPOINT_OWNERSHIP_SET,
    TWO_VCPU_CHECKPOINT_SECOND_ENTRY, TWO_VCPU_CHECKPOINT_SECOND_ID,
    TWO_VCPU_CHECKPOINT_SECOND_MARKER, TWO_VCPU_CHECKPOINT_SECOND_PROOF,
    TWO_VCPU_CHECKPOINT_SECOND_STACK, TWO_VCPU_CHECKPOINT_SHARED_MARKER,
};
use crate::vcpu::{PortIoDirection, Vcpu, VcpuExit};

const FIRST_STATUS: u8 = 0x01;
const SECOND_STATUS: u8 = 0x03;
const FIRST_CAPTURE_MARKER: u8 = b'A';
const SECOND_CAPTURE_MARKER: u8 = b'B';
const FIRST_CAPTURE_RIP: u64 = TWO_VCPU_CHECKPOINT_FIRST_ENTRY.get() + 14;
const SECOND_CAPTURE_RIP: u64 = TWO_VCPU_CHECKPOINT_SECOND_ENTRY.get() + 6;
const FIRST_COMPLETION_RIP: u64 = TWO_VCPU_CHECKPOINT_FIRST_ENTRY.get() + 35;
const SECOND_COMPLETION_RIP: u64 = TWO_VCPU_CHECKPOINT_SECOND_ENTRY.get() + 27;
const CORRUPT_FIRST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x12000);
const CORRUPT_SECOND_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x13000);
const CORRUPT_FIRST_STACK: u64 = 0x1fbff8;
const CORRUPT_SECOND_STACK: u64 = 0x1faff8;
const MP_STATE_RUNNABLE: u32 = 0;
const MP_STATE_UNINITIALIZED: u32 = 1;
const MP_STATE_HALTED: u32 = 3;
const IOAPIC_PIN: usize = 16;
const APIC_SPIV_OFFSET: usize = 0x0f0;
const APIC_LVT0_OFFSET: usize = 0x350;
const APIC_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;

#[rustfmt::skip]
const FIRST_GUEST_BYTES: [u8; 42] = [
    0xc6, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00, TWO_VCPU_CHECKPOINT_SHARED_MARKER,
    0x6a, TWO_VCPU_CHECKPOINT_FIRST_MARKER,
    0xb0, FIRST_CAPTURE_MARKER, 0xe6, 0xe9, 0x90,
    0x58, 0x3c, TWO_VCPU_CHECKPOINT_FIRST_MARKER, 0x75, 0x11,
    0x8a, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00,
    0x3c, TWO_VCPU_CHECKPOINT_SHARED_MARKER, 0x75, 0x06,
    0xb0, TWO_VCPU_CHECKPOINT_FIRST_MARKER, 0xe6, 0xe9, 0x90,
    0xf4,
    0xb0, b'F', 0xe6, 0xe9, 0xf4,
];

#[rustfmt::skip]
const SECOND_GUEST_BYTES: [u8; 34] = [
    0x6a, TWO_VCPU_CHECKPOINT_SECOND_MARKER,
    0xb0, SECOND_CAPTURE_MARKER, 0xe6, 0xe9, 0x90,
    0x58, 0x3c, TWO_VCPU_CHECKPOINT_SECOND_MARKER, 0x75, 0x11,
    0x8a, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00,
    0x3c, TWO_VCPU_CHECKPOINT_SHARED_MARKER, 0x75, 0x06,
    0xb0, TWO_VCPU_CHECKPOINT_SECOND_MARKER, 0xe6, 0xe9, 0x90,
    0xf4,
    0xb0, b'F', 0xe6, 0xe9, 0xf4,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuTwoDeviceTransactionGuestResult {
    mutation: BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison,
    restored: BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison,
    bars: [u64; 2],
    captured_statuses: [u8; 2],
    restored_statuses: [u8; 2],
    capture_pending: [bool; 2],
    reconstructed_pending: [bool; 2],
    first_capture_rip: u64,
    second_capture_rip: u64,
    first_completion_rip: u64,
    second_completion_rip: u64,
    first_proof: Vec<u8>,
    second_proof: Vec<u8>,
}

impl TwoVcpuTwoDeviceTransactionGuestResult {
    #[must_use]
    pub const fn mutation(&self) -> &BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison {
        &self.mutation
    }

    #[must_use]
    pub const fn restored(&self) -> &BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison {
        &self.restored
    }

    #[must_use]
    pub const fn bars(&self) -> [u64; 2] {
        self.bars
    }

    #[must_use]
    pub const fn captured_statuses(&self) -> [u8; 2] {
        self.captured_statuses
    }

    #[must_use]
    pub const fn restored_statuses(&self) -> [u8; 2] {
        self.restored_statuses
    }

    #[must_use]
    pub const fn capture_pending(&self) -> [bool; 2] {
        self.capture_pending
    }

    #[must_use]
    pub const fn reconstructed_pending(&self) -> [bool; 2] {
        self.reconstructed_pending
    }

    #[must_use]
    pub const fn first_capture_rip(&self) -> u64 {
        self.first_capture_rip
    }

    #[must_use]
    pub const fn second_capture_rip(&self) -> u64 {
        self.second_capture_rip
    }

    #[must_use]
    pub const fn first_completion_rip(&self) -> u64 {
        self.first_completion_rip
    }

    #[must_use]
    pub const fn second_completion_rip(&self) -> u64 {
        self.second_completion_rip
    }

    #[must_use]
    pub fn first_proof(&self) -> &[u8] {
        &self.first_proof
    }

    #[must_use]
    pub fn second_proof(&self) -> &[u8] {
        &self.second_proof
    }
}

pub fn run_two_vcpu_two_device_transaction_guest(
) -> Result<TwoVcpuTwoDeviceTransactionGuestResult, Error> {
    let first_image = FlatGuestImage::new(
        TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
        TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
        &FIRST_GUEST_BYTES,
    )?;
    let second_image = FlatGuestImage::new(
        TWO_VCPU_CHECKPOINT_SECOND_ENTRY,
        TWO_VCPU_CHECKPOINT_SECOND_ENTRY,
        &SECOND_GUEST_BYTES,
    )?;

    let backend = crate::kvm::KvmBackend::open()?;
    backend.require_mp_state_capability()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let first_layout = LongModeBootLayout::new(
        memory.region(),
        first_image.entry(),
        TWO_VCPU_CHECKPOINT_FIRST_STACK,
    )
    .expect("fixed first two-vCPU transaction layout remains valid");
    let second_layout = LongModeBootLayout::new(
        memory.region(),
        second_image.entry(),
        TWO_VCPU_CHECKPOINT_SECOND_STACK,
    )
    .expect("fixed second two-vCPU transaction layout remains valid");
    let corrupt_first_layout =
        LongModeBootLayout::new(memory.region(), CORRUPT_FIRST_ENTRY, CORRUPT_FIRST_STACK)
            .expect("fixed first two-vCPU transaction corruption layout remains valid");
    let corrupt_second_layout =
        LongModeBootLayout::new(memory.region(), CORRUPT_SECOND_ENTRY, CORRUPT_SECOND_STACK)
            .expect("fixed second two-vCPU transaction corruption layout remains valid");
    first_layout.install_page_tables(&mut memory)?;
    first_image.load(&mut memory)?;
    second_image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    let mut first = vm.create_vcpu(TWO_VCPU_CHECKPOINT_FIRST_ID)?;
    let mut second = vm.create_vcpu(TWO_VCPU_CHECKPOINT_SECOND_ID)?;
    first.initialize_long_mode(&first_layout)?;
    second.initialize_long_mode(&second_layout)?;
    let first_mp = first.ensure_runnable_mp_state()?;
    let second_mp = second.ensure_runnable_mp_state()?;
    if [first_mp, second_mp] != [MP_STATE_RUNNABLE, MP_STATE_RUNNABLE] {
        return Err(page_set_error(
            "two-vCPU two-device transaction MP-state preparation",
            format!("expected [0, 0], got [{first_mp}, {second_mp}]"),
        ));
    }

    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty two-vCPU two-device transaction MSR policy is valid");
    let mut mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_FIRST_BAR)
        .expect("fixed first transaction virtio-blk BAR remains available");
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_SECOND_BAR)
        .expect("fixed second transaction virtio-blk BAR remains available");
    let first_device = two_vcpu_two_device_prepared_device(
        TWO_HOST_REGISTRATION_FIRST_BAR,
        FIRST_STATUS,
    )?;
    let second_device = two_vcpu_two_device_prepared_device(
        TWO_HOST_REGISTRATION_SECOND_BAR,
        SECOND_STATUS,
    )?;
    mmio.restore_two_virtio_blk_checkpoints_atomic([
        (TWO_HOST_REGISTRATION_FIRST_BAR, &first_device),
        (TWO_HOST_REGISTRATION_SECOND_BAR, &second_device),
    ])?;

    let pair = default_two_host_registration_pair()?;
    let registrations = HostRegistrationPairCheckpoint::capture(pair).reconstruct(&backend, &vm)?;

    let (first_capture_rip, _) = two_vcpu_two_device_retired_barrier(
        &mut first,
        FIRST_CAPTURE_MARKER,
        FIRST_CAPTURE_RIP,
        "first two-vCPU two-device transaction capture barrier",
    )?;
    let (second_capture_rip, _) = two_vcpu_two_device_retired_barrier(
        &mut second,
        SECOND_CAPTURE_MARKER,
        SECOND_CAPTURE_RIP,
        "second two-vCPU two-device transaction capture barrier",
    )?;
    let capture_pending = registrations.pending_doorbells()?;
    if capture_pending != [false, false] {
        return two_vcpu_two_device_cleanup_error(
            registrations,
            &vm,
            page_set_error(
                "two-vCPU two-device transaction capture quiescence",
                format!("expected no pending accelerated doorbells, got {capture_pending:?}"),
            ),
        );
    }

    let transaction_result = TwoVcpuTwoDeviceCheckpointTransaction::capture(
        &first,
        &second,
        &vm,
        &msr_policy,
        &mmio,
        [
            TWO_HOST_REGISTRATION_SECOND_BAR,
            TWO_HOST_REGISTRATION_FIRST_BAR,
        ],
        &TWO_VCPU_CHECKPOINT_OWNERSHIP_SET,
        pair,
        &registrations,
    );
    let cleanup_result = registrations.deassign(&vm);
    let transaction = match (transaction_result, cleanup_result) {
        (Ok(transaction), Ok(())) => transaction,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(cleanup_error)) => return Err(cleanup_error),
        (Err(error), Err(cleanup_error)) => {
            return Err(page_set_error(
                "two-vCPU two-device transaction capture cleanup",
                format!("capture failed: {error}; registration cleanup also failed: {cleanup_error}"),
            ))
        }
    };

    let bars = transaction.checkpoint().device_bars();
    let captured_statuses = [
        transaction
            .checkpoint()
            .device(TWO_HOST_REGISTRATION_FIRST_BAR)
            .map(VirtioBlkDevice::status)
            .ok_or_else(|| {
                page_set_error(
                    "two-vCPU two-device transaction capture",
                    "first captured device disappeared",
                )
            })?,
        transaction
            .checkpoint()
            .device(TWO_HOST_REGISTRATION_SECOND_BAR)
            .map(VirtioBlkDevice::status)
            .ok_or_else(|| {
                page_set_error(
                    "two-vCPU two-device transaction capture",
                    "second captured device disappeared",
                )
            })?,
    ];
    if bars != [TWO_HOST_REGISTRATION_FIRST_BAR, TWO_HOST_REGISTRATION_SECOND_BAR]
        || captured_statuses != [FIRST_STATUS, SECOND_STATUS]
    {
        return Err(page_set_error(
            "two-vCPU two-device transaction capture contract",
            format!("unexpected bars/statuses: {bars:?} {captured_statuses:?}"),
        ));
    }

    two_vcpu_two_device_corrupt_pages(&mut vm)?;
    first.initialize_long_mode(&corrupt_first_layout)?;
    second.initialize_long_mode(&corrupt_second_layout)?;
    first.restore_multiprocessing_state_raw(MP_STATE_HALTED)?;
    second.restore_multiprocessing_state_raw(MP_STATE_UNINITIALIZED)?;
    two_vcpu_two_device_corrupt_controller(&first, &second, &vm)?;

    let first_corrupt = VirtioBlkDevice::new(TWO_HOST_REGISTRATION_FIRST_BAR);
    let second_corrupt = VirtioBlkDevice::new(TWO_HOST_REGISTRATION_SECOND_BAR);
    mmio.restore_two_virtio_blk_checkpoints_atomic([
        (TWO_HOST_REGISTRATION_FIRST_BAR, &first_corrupt),
        (TWO_HOST_REGISTRATION_SECOND_BAR, &second_corrupt),
    ])?;

    let mutation = transaction.checkpoint().verify(&first, &second, &vm, &mmio)?;
    two_vcpu_two_device_require_full_mismatch(&mutation)?;

    let (restored, reconstructed) = transaction.restore_and_reconstruct(
        &backend,
        &mut first,
        &mut second,
        &mut vm,
        &mut mmio,
    )?;
    if !restored.is_exact_match() {
        return two_vcpu_two_device_cleanup_error(
            reconstructed,
            &vm,
            page_set_error(
                "two-vCPU two-device transaction exact restore",
                "restored transaction did not compare exactly",
            ),
        );
    }
    let restored_statuses = match (
        mmio.virtio_blk_status_at(TWO_HOST_REGISTRATION_FIRST_BAR),
        mmio.virtio_blk_status_at(TWO_HOST_REGISTRATION_SECOND_BAR),
    ) {
        (Some(first_status), Some(second_status)) => [first_status, second_status],
        _ => {
            return two_vcpu_two_device_cleanup_error(
                reconstructed,
                &vm,
                page_set_error(
                    "two-vCPU two-device transaction status",
                    "one or both restored devices disappeared",
                ),
            )
        }
    };
    if restored_statuses != captured_statuses {
        return two_vcpu_two_device_cleanup_error(
            reconstructed,
            &vm,
            page_set_error(
                "two-vCPU two-device transaction device restore",
                format!(
                    "expected statuses {captured_statuses:?}, got {restored_statuses:?}"
                ),
            ),
        );
    }
    let reconstructed_pending = match reconstructed.pending_doorbells() {
        Ok(pending) => pending,
        Err(error) => {
            return two_vcpu_two_device_cleanup_error(reconstructed, &vm, error)
        }
    };
    if reconstructed_pending != [false, false] {
        return two_vcpu_two_device_cleanup_error(
            reconstructed,
            &vm,
            page_set_error(
                "two-vCPU two-device transaction reconstructed quiescence",
                format!(
                    "expected reconstructed pair to be quiescent, got {reconstructed_pending:?}"
                ),
            ),
        );
    }

    let proof_result = (|| -> Result<(u64, u64, Vec<u8>, Vec<u8>), Error> {
        let (first_completion_rip, _) = two_vcpu_two_device_retired_barrier(
            &mut first,
            TWO_VCPU_CHECKPOINT_FIRST_PROOF[0],
            FIRST_COMPLETION_RIP,
            "first two-vCPU two-device transaction completion",
        )?;
        let (second_completion_rip, _) = two_vcpu_two_device_retired_barrier(
            &mut second,
            TWO_VCPU_CHECKPOINT_SECOND_PROOF[0],
            SECOND_COMPLETION_RIP,
            "second two-vCPU two-device transaction completion",
        )?;
        Ok((
            first_completion_rip,
            second_completion_rip,
            TWO_VCPU_CHECKPOINT_FIRST_PROOF.to_vec(),
            TWO_VCPU_CHECKPOINT_SECOND_PROOF.to_vec(),
        ))
    })();
    let cleanup_result = reconstructed.deassign(&vm);
    let (first_completion_rip, second_completion_rip, first_proof, second_proof) =
        match (proof_result, cleanup_result) {
            (Ok(proof), Ok(())) => proof,
            (Err(error), Ok(())) => return Err(error),
            (Ok(_), Err(cleanup_error)) => return Err(cleanup_error),
            (Err(error), Err(cleanup_error)) => {
                return Err(page_set_error(
                    "two-vCPU two-device transaction reconstructed cleanup",
                    format!("resume failed: {error}; cleanup also failed: {cleanup_error}"),
                ))
            }
        };

    Ok(TwoVcpuTwoDeviceTransactionGuestResult {
        mutation,
        restored,
        bars,
        captured_statuses,
        restored_statuses,
        capture_pending,
        reconstructed_pending,
        first_capture_rip,
        second_capture_rip,
        first_completion_rip,
        second_completion_rip,
        first_proof,
        second_proof,
    })
}

fn two_vcpu_two_device_prepared_device(bar: u64, status: u8) -> Result<VirtioBlkDevice, Error> {
    let mut device = VirtioBlkDevice::new(bar);
    device.write(0x14, &[0x01]).map_err(|error| {
        page_set_error(
            "prepare two-vCPU transaction virtio-blk device",
            error.to_string(),
        )
    })?;
    if status == 0x03 {
        device.write(0x14, &[0x03]).map_err(|error| {
            page_set_error(
                "prepare two-vCPU transaction virtio-blk device",
                error.to_string(),
            )
        })?;
    }
    if device.status() != status || !device.checkpoint_quiescent() {
        return Err(page_set_error(
            "prepare two-vCPU transaction virtio-blk device",
            format!(
                "prepared BAR {bar:#x} status {}, expected {status}",
                device.status()
            ),
        ));
    }
    Ok(device)
}

fn two_vcpu_two_device_retired_barrier(
    vcpu: &mut Vcpu,
    expected: u8,
    expected_rip: u64,
    stage: &'static str,
) -> Result<(u64, u64), Error> {
    let mut port_io = PortIoBus::with_debug_port();
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Io {
        return Err(page_set_error(
            stage,
            format!("vCPU {} expected KVM_EXIT_IO marker, got {exit:?}", vcpu.id().get()),
        ));
    }
    let io_exit = vcpu.port_io_exit()?;
    if io_exit.direction() != PortIoDirection::Out
        || io_exit.port() != DEBUG_PORT
        || io_exit.size() != 1
        || io_exit.count() != 1
        || io_exit.output_data() != [expected]
    {
        return Err(page_set_error(
            stage,
            format!(
                "vCPU {} unexpected debug marker {io_exit:?}; expected {expected:#x}",
                vcpu.id().get()
            ),
        ));
    }
    if port_io.dispatch(&io_exit)? != PortIoService::Output {
        return Err(page_set_error(
            stage,
            format!("vCPU {} debug marker requested input", vcpu.id().get()),
        ));
    }

    vcpu.set_guest_single_step(true)?;
    let step_result = vcpu.run_once();
    let disable_result = vcpu.set_guest_single_step(false);
    let step_exit = match (step_result, disable_result) {
        (Ok(exit), Ok(())) => exit,
        (Err(error), _) | (Ok(_), Err(error)) => return Err(error),
    };
    if step_exit != VcpuExit::Debug {
        return Err(page_set_error(
            stage,
            format!(
                "vCPU {} expected KVM_EXIT_DEBUG after retiring marker, got {step_exit:?}",
                vcpu.id().get()
            ),
        ));
    }
    let registers = vcpu.registers()?;
    if registers.rip != expected_rip || registers.rflags & 0x2 != 0x2 {
        return Err(page_set_error(
            stage,
            format!(
                "vCPU {} expected rip={expected_rip:#x} with architectural bit1, got rip={:#x} rflags={:#x}",
                vcpu.id().get(),
                registers.rip,
                registers.rflags
            ),
        ));
    }
    Ok((registers.rip, registers.rflags))
}

fn two_vcpu_two_device_corrupt_pages(vm: &mut crate::kvm::Vm) -> Result<(), Error> {
    let memory = vm.guest_memory_mut().ok_or_else(|| {
        page_set_error(
            "two-vCPU two-device transaction page corruption",
            "VM lost registered guest memory",
        )
    })?;
    for (index, address) in TWO_VCPU_CHECKPOINT_OWNERSHIP_SET.iter().enumerate() {
        memory.write(
            *address,
            &vec![0xa5_u8.wrapping_add(index as u8); LONG_MODE_PAGE_SIZE as usize],
        )?;
    }
    Ok(())
}

fn two_vcpu_two_device_corrupt_controller(
    first: &Vcpu,
    second: &Vcpu,
    vm: &crate::kvm::Vm,
) -> Result<(), Error> {
    let master = vm.capture_master_pic_state()?;
    vm.restore_master_pic_state(&master.with_imr(master.imr() ^ 0x01))?;
    let slave = vm.capture_slave_pic_state()?;
    vm.restore_slave_pic_state(&slave.with_imr(slave.imr() ^ 0x02))?;

    let ioapic = vm.capture_ioapic_state()?;
    let entry = ioapic
        .redirection_entry(IOAPIC_PIN)
        .expect("fixed two-vCPU transaction IOAPIC pin remains valid");
    let corrupt_ioapic = ioapic
        .with_redirection_entry(IOAPIC_PIN, entry ^ (1_u64 << 16))
        .expect("fixed two-vCPU transaction IOAPIC pin remains valid");
    vm.restore_ioapic_state(&corrupt_ioapic)?;

    let mut first_lapic = first.capture_lapic_checkpoint_state()?;
    let first_lvt0 = two_vcpu_two_device_read_lapic(&first_lapic, APIC_LVT0_OFFSET);
    two_vcpu_two_device_write_lapic(
        &mut first_lapic,
        APIC_LVT0_OFFSET,
        first_lvt0 ^ APIC_LVT_MASKED,
    );
    first.restore_lapic_checkpoint_state(&first_lapic)?;

    let mut second_lapic = second.capture_lapic_checkpoint_state()?;
    let second_spiv = two_vcpu_two_device_read_lapic(&second_lapic, APIC_SPIV_OFFSET);
    two_vcpu_two_device_write_lapic(
        &mut second_lapic,
        APIC_SPIV_OFFSET,
        second_spiv ^ APIC_SOFTWARE_ENABLE,
    );
    second.restore_lapic_checkpoint_state(&second_lapic)?;
    Ok(())
}

fn two_vcpu_two_device_require_full_mismatch(
    comparison: &BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison,
) -> Result<(), Error> {
    let controller = comparison.controller();
    for page in TWO_VCPU_CHECKPOINT_OWNERSHIP_SET {
        if controller.page_exact(page) != Some(false) {
            return Err(page_set_error(
                "two-vCPU two-device transaction corruption proof",
                format!("owned page {:#x} did not mismatch", page.get()),
            ));
        }
    }
    for id in [TWO_VCPU_CHECKPOINT_FIRST_ID, TWO_VCPU_CHECKPOINT_SECOND_ID] {
        if controller.vcpu_exact(id) != Some(false)
            || controller.mp_state_exact(id) != Some(false)
            || controller.lapic_exact(id) != Some(false)
        {
            return Err(page_set_error(
                "two-vCPU two-device transaction corruption proof",
                format!(
                    "vCPU {} mismatch evidence incomplete: vcpu={:?} mp={:?} lapic={:?}",
                    id.get(),
                    controller.vcpu_exact(id),
                    controller.mp_state_exact(id),
                    controller.lapic_exact(id)
                ),
            ));
        }
    }
    if controller.master_pic_exact()
        || controller.slave_pic_exact()
        || controller.ioapic_exact()
        || comparison.device_exact(TWO_HOST_REGISTRATION_FIRST_BAR) != Some(false)
        || comparison.device_exact(TWO_HOST_REGISTRATION_SECOND_BAR) != Some(false)
        || comparison.is_exact_match()
    {
        return Err(page_set_error(
            "two-vCPU two-device transaction corruption proof",
            format!(
                "expected controller/devices to mismatch: master={} slave={} ioapic={} first={:?} second={:?}",
                controller.master_pic_exact(),
                controller.slave_pic_exact(),
                controller.ioapic_exact(),
                comparison.device_exact(TWO_HOST_REGISTRATION_FIRST_BAR),
                comparison.device_exact(TWO_HOST_REGISTRATION_SECOND_BAR)
            ),
        ));
    }
    Ok(())
}

fn two_vcpu_two_device_cleanup_error(
    registrations: ReconstructedHostRegistrationPair,
    vm: &crate::kvm::Vm,
    primary: Error,
) -> Result<TwoVcpuTwoDeviceTransactionGuestResult, Error> {
    match registrations.deassign(vm) {
        Ok(()) => Err(primary),
        Err(cleanup_error) => Err(page_set_error(
            "two-vCPU two-device transaction cleanup",
            format!("{primary}; cleanup also failed: {cleanup_error}"),
        )),
    }
}

fn two_vcpu_two_device_read_lapic(
    state: &crate::kvm::sys::KvmLapicState,
    offset: usize,
) -> u32 {
    u32::from_le_bytes(
        state.regs[offset..offset + 4]
            .try_into()
            .expect("fixed LAPIC register offset remains valid"),
    )
}

fn two_vcpu_two_device_write_lapic(
    state: &mut crate::kvm::sys::KvmLapicState,
    offset: usize,
    value: u32,
) {
    state.regs[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_guest_has_retired_capture_and_completion_barriers() {
        assert_eq!(FIRST_GUEST_BYTES.len(), 42);
        assert_eq!(SECOND_GUEST_BYTES.len(), 34);
        assert_eq!(&FIRST_GUEST_BYTES[10..15], &[0xb0, b'A', 0xe6, 0xe9, 0x90]);
        assert_eq!(&SECOND_GUEST_BYTES[2..7], &[0xb0, b'B', 0xe6, 0xe9, 0x90]);
        assert_eq!(FIRST_CAPTURE_RIP, 0x1000e);
        assert_eq!(SECOND_CAPTURE_RIP, 0x11006);
        assert_eq!(FIRST_COMPLETION_RIP, 0x10023);
        assert_eq!(SECOND_COMPLETION_RIP, 0x1101b);
    }

    #[test]
    fn transaction_device_states_are_distinct_and_quiescent() {
        let first =
            two_vcpu_two_device_prepared_device(TWO_HOST_REGISTRATION_FIRST_BAR, FIRST_STATUS)
                .unwrap();
        let second =
            two_vcpu_two_device_prepared_device(TWO_HOST_REGISTRATION_SECOND_BAR, SECOND_STATUS)
                .unwrap();
        assert!(first.checkpoint_quiescent());
        assert!(second.checkpoint_quiescent());
        assert_eq!(first.status(), FIRST_STATUS);
        assert_eq!(second.status(), SECOND_STATUS);
    }
}
