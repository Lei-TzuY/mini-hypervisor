use super::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::error::{Error, HostEnvironmentError};
use crate::interrupt::{LongModeInterruptLayout, X86_RFLAGS_INTERRUPT_ENABLE};
use crate::kvm::sys::KvmMsiMessage;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId};
use std::io;
use std::sync::mpsc;

const RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const FIRST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
const SECOND_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_1000);
const HANDLER_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_2000);
const FIRST_STACK: u64 = 0x1f_f000;
const SECOND_STACK: u64 = 0x1f_e000;
const MSI_ADDRESS_BASE: u64 = 0xfee0_0000;
const MSI_DESTINATION_SHIFT: u32 = 12;
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;

pub const FIRST_VCPU_ID: VcpuId = VcpuId::BOOT;
pub const SECOND_VCPU_ID: VcpuId = VcpuId::new(1);
pub const TARGET_VECTOR: u8 = 0x51;
pub const TARGET_MSI_ADDRESS: u64 =
    MSI_ADDRESS_BASE | ((SECOND_VCPU_ID.get() as u64) << MSI_DESTINATION_SHIFT);
pub const TARGET_MSI_DATA: u32 = TARGET_VECTOR as u32;
pub const FIRST_PROOF: &[u8; 3] = b"0MD";
pub const SECOND_PROOF: &[u8; 4] = b"RI1D";

const FIRST_GUEST_BYTES: [u8; 15] = [
    0xfb, // sti
    0x90, // nop -- complete STI shadow before the isolation barrier
    0xb0, b'0', 0xe6, 0xe9, // host synchronization barrier
    0xb0, b'M', 0xe6, 0xe9, // mainline proof; wrong-target MSI emits I before this
    0xb0, b'D', 0xe6, 0xe9, // completion barrier
    0xf4, // unreachable terminal HLT after userspace accepts D
];

const SECOND_GUEST_BYTES: [u8; 16] = [
    0xfa, // cli -- readiness barrier must observe IF clear
    0xb0, b'R', 0xe6, 0xe9, // worker readiness barrier
    0xfb, // sti
    0x90, // nop -- complete STI shadow before the mainline can emit 1
    0xb0, b'1', 0xe6, 0xe9, // must be preceded by the targeted MSI handler byte I
    0xb0, b'D', 0xe6, 0xe9, // worker completion barrier
    0xf4, // unreachable terminal HLT after userspace accepts D
];

const HANDLER_BYTES: [u8; 6] = [
    0xb0, b'I', 0xe6, 0xe9, // targeted MSI handler identity
    0x48, 0xcf, // iretq
];

// SAFETY: `Vcpu` uniquely owns both its vCPU fd and its `kvm_run` mapping. No pointer into the
// mapping escapes the object, every operation that can mutate `kvm_run` requires exclusive
// `&mut Vcpu`, and moving the owned fd/mapping to another userspace thread does not create an
// alias or concurrent KVM_RUN. This milestone intentionally establishes only ownership transfer;
// `Vcpu` remains non-`Sync` and is never shared between threads.
unsafe impl Send for Vcpu {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuTargetedMsiResult {
    first_io_exits: Vec<PortIoExit>,
    second_io_exits: Vec<PortIoExit>,
    first_proof: Vec<u8>,
    second_proof: Vec<u8>,
    first_barrier_rflags: u64,
    second_ready_rflags: u64,
    second_completion_rflags: u64,
    msi_address: u64,
    msi_data: u32,
    msi_delivery_count: u32,
}

impl TwoVcpuTargetedMsiResult {
    #[must_use]
    pub fn first_io_exits(&self) -> &[PortIoExit] {
        &self.first_io_exits
    }

    #[must_use]
    pub fn second_io_exits(&self) -> &[PortIoExit] {
        &self.second_io_exits
    }

    #[must_use]
    pub fn first_proof(&self) -> &[u8] {
        &self.first_proof
    }

    #[must_use]
    pub fn second_proof(&self) -> &[u8] {
        &self.second_proof
    }

    #[must_use]
    pub const fn first_barrier_rflags(&self) -> u64 {
        self.first_barrier_rflags
    }

    #[must_use]
    pub const fn second_ready_rflags(&self) -> u64 {
        self.second_ready_rflags
    }

    #[must_use]
    pub const fn second_completion_rflags(&self) -> u64 {
        self.second_completion_rflags
    }

    #[must_use]
    pub const fn msi_address(&self) -> u64 {
        self.msi_address
    }

    #[must_use]
    pub const fn msi_data(&self) -> u32 {
        self.msi_data
    }

    #[must_use]
    pub const fn msi_delivery_count(&self) -> u32 {
        self.msi_delivery_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerCommand {
    Continue,
    Abort,
}

#[derive(Debug)]
struct SecondVcpuResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    ready_rflags: u64,
    completion_rflags: u64,
}

pub fn run_two_vcpu_targeted_msi_guest() -> Result<TwoVcpuTargetedMsiResult, Error> {
    let first_image = FlatGuestImage::new(FIRST_ENTRY, FIRST_ENTRY, &FIRST_GUEST_BYTES)?;
    let second_image = FlatGuestImage::new(SECOND_ENTRY, SECOND_ENTRY, &SECOND_GUEST_BYTES)?;
    let handler_image = FlatGuestImage::new(HANDLER_ENTRY, HANDLER_ENTRY, &HANDLER_BYTES)?;

    let backend = KvmBackend::open()?;
    backend.require_signal_msi_capability()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(RAM_BASE, LONG_MODE_IDENTITY_MAP_SIZE)?;
    let first_layout = LongModeInterruptLayout::new(
        memory.region(),
        first_image.entry(),
        FIRST_STACK,
        TARGET_VECTOR,
        handler_image.entry(),
    )
    .expect("fixed first targeted-MSI layout remains valid");
    let second_layout = LongModeInterruptLayout::new(
        memory.region(),
        second_image.entry(),
        SECOND_STACK,
        TARGET_VECTOR,
        handler_image.entry(),
    )
    .expect("fixed second targeted-MSI layout remains valid");
    first_layout.install_tables(&mut memory)?;
    first_image.load(&mut memory)?;
    second_image.load(&mut memory)?;
    handler_image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    let mut first_vcpu = vm.create_vcpu(FIRST_VCPU_ID)?;
    let mut second_vcpu = vm.create_vcpu(SECOND_VCPU_ID)?;
    first_vcpu.initialize_long_mode_interrupts(&first_layout)?;
    second_vcpu.initialize_long_mode_interrupts(&second_layout)?;
    let _ = first_vcpu.configure_legacy_pic_extint()?;
    let _ = second_vcpu.configure_legacy_pic_extint()?;

    let (ready_tx, ready_rx) = mpsc::channel::<u64>();
    let (command_tx, command_rx) = mpsc::channel::<WorkerCommand>();
    let worker = std::thread::spawn(move || -> Result<SecondVcpuResult, Error> {
        let mut port_io = PortIoBus::with_debug_port();
        let readiness = run_expected_debug_output(
            &mut second_vcpu,
            &mut port_io,
            b'R',
            "targeted MSI second-vCPU readiness",
        )?;
        let ready_rflags = second_vcpu.registers()?.rflags;
        require_interrupt_disabled_flags(
            SECOND_VCPU_ID,
            "targeted MSI second-vCPU readiness state",
            ready_rflags,
        )?;
        ready_tx.send(ready_rflags).map_err(|_| {
            verification_error(
                SECOND_VCPU_ID,
                "targeted MSI worker readiness channel",
                "main thread dropped readiness receiver",
            )
        })?;

        match command_rx.recv().map_err(|_| {
            verification_error(
                SECOND_VCPU_ID,
                "targeted MSI worker command channel",
                "main thread dropped worker command sender",
            )
        })? {
            WorkerCommand::Continue => {}
            WorkerCommand::Abort => {
                return Err(verification_error(
                    SECOND_VCPU_ID,
                    "targeted MSI worker abort",
                    "host MSI delivery failed before worker resume",
                ));
            }
        }

        let handler = run_expected_debug_output(
            &mut second_vcpu,
            &mut port_io,
            b'I',
            "targeted MSI second-vCPU handler",
        )?;
        let mainline = run_expected_debug_output(
            &mut second_vcpu,
            &mut port_io,
            b'1',
            "targeted MSI second-vCPU mainline",
        )?;
        let completion = run_expected_debug_output(
            &mut second_vcpu,
            &mut port_io,
            b'D',
            "targeted MSI second-vCPU completion",
        )?;
        let completion_rflags = second_vcpu.registers()?.rflags;
        require_interrupt_enabled_flags(
            SECOND_VCPU_ID,
            "targeted MSI second-vCPU completion state",
            completion_rflags,
        )?;
        let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
        if proof.as_slice() != SECOND_PROOF {
            return Err(verification_error(
                SECOND_VCPU_ID,
                "targeted MSI second-vCPU proof",
                format!("expected {:?}, got {proof:?}", SECOND_PROOF),
            ));
        }
        Ok(SecondVcpuResult {
            io_exits: vec![readiness, handler, mainline, completion],
            proof,
            ready_rflags,
            completion_rflags,
        })
    });

    let worker_ready_rflags = ready_rx.recv().map_err(|_| {
        join_worker_failure(
            worker.thread().id(),
            "targeted MSI worker exited before readiness",
        )
    })?;
    require_interrupt_disabled_flags(
        SECOND_VCPU_ID,
        "targeted MSI worker readiness readback",
        worker_ready_rflags,
    )?;

    let mut first_port_io = PortIoBus::with_debug_port();
    let first_barrier = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'0',
        "targeted MSI first-vCPU isolation barrier",
    )?;
    let first_barrier_rflags = first_vcpu.registers()?.rflags;
    require_interrupt_enabled_flags(
        FIRST_VCPU_ID,
        "targeted MSI first-vCPU barrier state",
        first_barrier_rflags,
    )?;

    let delivery = vm.signal_msi(KvmMsiMessage::new(TARGET_MSI_ADDRESS, TARGET_MSI_DATA));
    let delivery_count = match delivery {
        Ok(count) if count == 1 => count,
        Ok(count) => {
            let _ = command_tx.send(WorkerCommand::Abort);
            let _ = worker.join();
            return Err(verification_error(
                FIRST_VCPU_ID,
                "targeted MSI delivery count",
                format!("expected exactly one MSI delivery, got {count}"),
            ));
        }
        Err(error) => {
            let _ = command_tx.send(WorkerCommand::Abort);
            let _ = worker.join();
            return Err(error);
        }
    };

    command_tx.send(WorkerCommand::Continue).map_err(|_| {
        verification_error(
            FIRST_VCPU_ID,
            "targeted MSI worker resume channel",
            "worker exited before resume command",
        )
    })?;
    let second = worker.join().map_err(|_| {
        verification_error(
            SECOND_VCPU_ID,
            "targeted MSI worker join",
            "second-vCPU worker panicked",
        )
    })??;

    let first_mainline = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'M',
        "targeted MSI first-vCPU post-delivery isolation",
    )?;
    let first_completion = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'D',
        "targeted MSI first-vCPU completion",
    )?;
    let first_completion_rflags = first_vcpu.registers()?.rflags;
    require_interrupt_enabled_flags(
        FIRST_VCPU_ID,
        "targeted MSI first-vCPU completion state",
        first_completion_rflags,
    )?;
    let first_proof = first_port_io.debug_output().unwrap_or(&[]).to_vec();
    if first_proof.as_slice() != FIRST_PROOF {
        return Err(verification_error(
            FIRST_VCPU_ID,
            "targeted MSI first-vCPU proof",
            format!("expected {:?}, got {first_proof:?}", FIRST_PROOF),
        ));
    }

    Ok(TwoVcpuTargetedMsiResult {
        first_io_exits: vec![first_barrier, first_mainline, first_completion],
        second_io_exits: second.io_exits,
        first_proof,
        second_proof: second.proof,
        first_barrier_rflags,
        second_ready_rflags: second.ready_rflags,
        second_completion_rflags: second.completion_rflags,
        msi_address: TARGET_MSI_ADDRESS,
        msi_data: TARGET_MSI_DATA,
        msi_delivery_count: delivery_count,
    })
}

fn run_expected_debug_output(
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    expected: u8,
    stage: &'static str,
) -> Result<PortIoExit, Error> {
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Io {
        return Err(verification_error(
            vcpu.id(),
            stage,
            format!("expected KVM_EXIT_IO, got {exit:?}"),
        ));
    }
    let io_exit = vcpu.port_io_exit()?;
    if io_exit.direction() != PortIoDirection::Out
        || io_exit.port() != DEBUG_PORT
        || io_exit.size() != 1
        || io_exit.count() != 1
        || io_exit.output_data() != [expected]
    {
        return Err(verification_error(
            vcpu.id(),
            stage,
            format!("unexpected debug output exit: {io_exit:?}; expected byte {expected:#x}"),
        ));
    }
    if port_io.dispatch(&io_exit)? != PortIoService::Output {
        return Err(verification_error(
            vcpu.id(),
            stage,
            "debug output unexpectedly requested an input response",
        ));
    }
    Ok(io_exit)
}

fn require_interrupt_disabled_flags(
    id: VcpuId,
    stage: &'static str,
    rflags: u64,
) -> Result<(), Error> {
    if rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || rflags & X86_RFLAGS_INTERRUPT_ENABLE != 0
    {
        return Err(verification_error(
            id,
            stage,
            format!("expected architectural bit1 set and IF clear, got RFLAGS {rflags:#x}"),
        ));
    }
    Ok(())
}

fn require_interrupt_enabled_flags(
    id: VcpuId,
    stage: &'static str,
    rflags: u64,
) -> Result<(), Error> {
    if rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || rflags & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
    {
        return Err(verification_error(
            id,
            stage,
            format!("expected architectural bit1 and IF set, got RFLAGS {rflags:#x}"),
        ));
    }
    Ok(())
}

fn verification_error(id: VcpuId, operation: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: id.get(),
        operation,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

fn join_worker_failure(_thread_id: std::thread::ThreadId, detail: &'static str) -> Error {
    verification_error(SECOND_VCPU_ID, "targeted MSI worker readiness", detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn vcpu_ownership_can_move_between_threads_without_becoming_shared() {
        assert_send::<Vcpu>();
    }

    #[test]
    fn fixed_target_is_apic_id_one_and_vector_51() {
        assert_eq!(SECOND_VCPU_ID.get(), 1);
        assert_eq!(TARGET_MSI_ADDRESS, 0xfee0_1000);
        assert_eq!(TARGET_MSI_DATA, 0x51);
        assert_eq!(TARGET_VECTOR, 0x51);
    }

    #[test]
    fn fixed_guest_programs_make_wrong_target_and_missing_target_observable() {
        assert_eq!(&FIRST_GUEST_BYTES[..2], &[0xfb, 0x90]);
        assert_eq!(&FIRST_GUEST_BYTES[2..6], &[0xb0, b'0', 0xe6, 0xe9]);
        assert_eq!(&SECOND_GUEST_BYTES[..5], &[0xfa, 0xb0, b'R', 0xe6, 0xe9]);
        assert_eq!(&SECOND_GUEST_BYTES[5..7], &[0xfb, 0x90]);
        assert_eq!(HANDLER_BYTES, [0xb0, b'I', 0xe6, 0xe9, 0x48, 0xcf]);
        assert_eq!(FIRST_PROOF, b"0MD");
        assert_eq!(SECOND_PROOF, b"RI1D");
    }
}
