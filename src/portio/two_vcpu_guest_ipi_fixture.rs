use super::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::error::{Error, HostEnvironmentError};
use crate::interrupt::{LongModeInterruptLayout, X86_RFLAGS_INTERRUPT_ENABLE};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::mmio::long_mode::{LongModeMmioBootLayout, LongModeMmioPageMapping};
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId};
use std::io;
use std::sync::mpsc;

const RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const FIRST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
const SECOND_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_1000);
const HANDLER_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_2000);
const FIRST_STACK: u64 = 0x1f_f000;
const SECOND_STACK: u64 = 0x1f_e000;
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;

pub const FIRST_VCPU_ID: VcpuId = VcpuId::BOOT;
pub const SECOND_VCPU_ID: VcpuId = VcpuId::new(1);
pub const LAPIC_VIRTUAL_PAGE: u64 = 0x50_0000;
pub const LAPIC_GPA: u64 = 0xfee0_0000;
pub const LAPIC_ICR_LOW_OFFSET: u32 = 0x300;
pub const LAPIC_ICR_HIGH_OFFSET: u32 = 0x310;
pub const LAPIC_EOI_OFFSET: u32 = 0x0b0;
pub const TARGET_APIC_ID: u8 = 1;
pub const TARGET_VECTOR: u8 = 0x52;
pub const ICR_HIGH_VALUE: u32 = (TARGET_APIC_ID as u32) << 24;
pub const ICR_LOW_VALUE: u32 = TARGET_VECTOR as u32;
pub const FIRST_PROOF: &[u8; 4] = b"0SMD";
pub const SECOND_PROOF: &[u8; 4] = b"RI1D";

const FIRST_GUEST_BYTES: [u8; 49] = [
    0xfb, // sti
    0x90, // nop -- complete STI shadow before the isolation barrier
    0x48,
    0xbb,
    0x00,
    0x00,
    0x50,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00, // movabs $0x500000, %rbx
    0xb0,
    b'0',
    0xe6,
    0xe9, // synchronization/isolation barrier before guest ICR writes
    0xc7,
    0x83,
    0x10,
    0x03,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x01, // ICR high: destination APIC ID 1
    0xc7,
    0x83,
    0x00,
    0x03,
    0x00,
    0x00,
    TARGET_VECTOR,
    0x00,
    0x00,
    0x00, // ICR low: fixed vector 0x52
    0xb0,
    b'S',
    0xe6,
    0xe9, // proves both LAPIC MMIO writes completed in-kernel
    0xb0,
    b'M',
    0xe6,
    0xe9, // wrong-target IPI must not emit I before this
    0xb0,
    b'D',
    0xe6,
    0xe9, // first-vCPU completion
    0xf4,
];

const SECOND_GUEST_BYTES: [u8; 26] = [
    0xfa, // cli -- readiness must observe IF clear
    0x48, 0xbb, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs LAPIC alias, %rbx
    0xb0, b'R', 0xe6, 0xe9, // worker readiness barrier
    0xfb, // sti
    0x90, // complete STI shadow before mainline byte 1
    0xb0, b'1', 0xe6, 0xe9, 0xb0, b'D', 0xe6, 0xe9, 0xf4,
];

const HANDLER_BYTES: [u8; 16] = [
    0xb0, b'I', 0xe6, 0xe9, // handler identity
    0xc7, 0x83, 0xb0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, // movl $0, 0xb0(%rbx): LAPIC EOI
    0x48, 0xcf, // iretq
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuGuestIpiResult {
    first_io_exits: Vec<PortIoExit>,
    second_io_exits: Vec<PortIoExit>,
    first_proof: Vec<u8>,
    second_proof: Vec<u8>,
    first_barrier_rflags: u64,
    first_send_rflags: u64,
    first_completion_rflags: u64,
    second_ready_rflags: u64,
    second_completion_rflags: u64,
    second_mp_state: u32,
}

impl TwoVcpuGuestIpiResult {
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
    pub const fn first_send_rflags(&self) -> u64 {
        self.first_send_rflags
    }
    #[must_use]
    pub const fn first_completion_rflags(&self) -> u64 {
        self.first_completion_rflags
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
    pub const fn second_mp_state(&self) -> u32 {
        self.second_mp_state
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

pub fn run_two_vcpu_guest_ipi() -> Result<TwoVcpuGuestIpiResult, Error> {
    let first_image = FlatGuestImage::new(FIRST_ENTRY, FIRST_ENTRY, &FIRST_GUEST_BYTES)?;
    let second_image = FlatGuestImage::new(SECOND_ENTRY, SECOND_ENTRY, &SECOND_GUEST_BYTES)?;
    let handler_image = FlatGuestImage::new(HANDLER_ENTRY, HANDLER_ENTRY, &HANDLER_BYTES)?;

    let backend = KvmBackend::open()?;
    backend.require_mp_state_capability()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(RAM_BASE, LONG_MODE_IDENTITY_MAP_SIZE)?;
    let first_layout = LongModeInterruptLayout::new(
        memory.region(),
        first_image.entry(),
        FIRST_STACK,
        TARGET_VECTOR,
        handler_image.entry(),
    )
    .expect("fixed first guest-IPI layout remains valid");
    let second_layout = LongModeInterruptLayout::new(
        memory.region(),
        second_image.entry(),
        SECOND_STACK,
        TARGET_VECTOR,
        handler_image.entry(),
    )
    .expect("fixed second guest-IPI layout remains valid");
    let lapic_mapping = LongModeMmioBootLayout::with_device_mappings(
        memory.region(),
        first_image.entry(),
        FIRST_STACK,
        vec![LongModeMmioPageMapping::new(LAPIC_VIRTUAL_PAGE, LAPIC_GPA)],
    )
    .expect("fixed LAPIC alias mapping remains valid");
    first_layout.install_tables(&mut memory)?;
    lapic_mapping.install_page_tables(&mut memory)?;
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
    let second_mp_state = second_vcpu.ensure_runnable_mp_state()?;

    let (ready_tx, ready_rx) = mpsc::channel::<u64>();
    let (command_tx, command_rx) = mpsc::channel::<WorkerCommand>();
    let worker = std::thread::spawn(move || -> Result<SecondVcpuResult, Error> {
        let mut port_io = PortIoBus::with_debug_port();
        let readiness = run_expected_debug_output(
            &mut second_vcpu,
            &mut port_io,
            b'R',
            "guest IPI second-vCPU readiness",
        )?;
        let ready_rflags = second_vcpu.registers()?.rflags;
        require_interrupt_disabled_flags(
            SECOND_VCPU_ID,
            "guest IPI second-vCPU readiness state",
            ready_rflags,
        )?;
        ready_tx.send(ready_rflags).map_err(|_| {
            verification_error(
                SECOND_VCPU_ID,
                "guest IPI worker readiness channel",
                "main thread dropped readiness receiver",
            )
        })?;
        match command_rx.recv().map_err(|_| {
            verification_error(
                SECOND_VCPU_ID,
                "guest IPI worker command channel",
                "main thread dropped command sender",
            )
        })? {
            WorkerCommand::Continue => {}
            WorkerCommand::Abort => {
                return Err(verification_error(
                    SECOND_VCPU_ID,
                    "guest IPI worker abort",
                    "guest ICR delivery failed before worker resume",
                ))
            }
        }
        let handler = run_expected_debug_output(
            &mut second_vcpu,
            &mut port_io,
            b'I',
            "guest IPI second-vCPU handler",
        )?;
        let mainline = run_expected_debug_output(
            &mut second_vcpu,
            &mut port_io,
            b'1',
            "guest IPI second-vCPU mainline",
        )?;
        let completion = run_expected_debug_output(
            &mut second_vcpu,
            &mut port_io,
            b'D',
            "guest IPI second-vCPU completion",
        )?;
        let completion_rflags = second_vcpu.registers()?.rflags;
        require_interrupt_enabled_flags(
            SECOND_VCPU_ID,
            "guest IPI second-vCPU completion state",
            completion_rflags,
        )?;
        let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
        if proof.as_slice() != SECOND_PROOF {
            return Err(verification_error(
                SECOND_VCPU_ID,
                "guest IPI second-vCPU proof",
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

    let worker_ready_rflags = ready_rx
        .recv()
        .map_err(|_| join_worker_failure("guest IPI worker exited before readiness"))?;
    require_interrupt_disabled_flags(
        SECOND_VCPU_ID,
        "guest IPI worker readiness readback",
        worker_ready_rflags,
    )?;

    let mut first_port_io = PortIoBus::with_debug_port();
    let first_barrier = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'0',
        "guest IPI first-vCPU pre-ICR barrier",
    )?;
    let first_barrier_rflags = first_vcpu.registers()?.rflags;
    require_interrupt_enabled_flags(
        FIRST_VCPU_ID,
        "guest IPI first-vCPU pre-ICR state",
        first_barrier_rflags,
    )?;

    let first_send = match run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'S',
        "guest IPI first-vCPU post-ICR barrier",
    ) {
        Ok(exit) => exit,
        Err(error) => {
            let _ = command_tx.send(WorkerCommand::Abort);
            let _ = worker.join();
            return Err(error);
        }
    };
    let first_send_rflags = first_vcpu.registers()?.rflags;
    require_interrupt_enabled_flags(
        FIRST_VCPU_ID,
        "guest IPI first-vCPU post-ICR state",
        first_send_rflags,
    )?;

    command_tx.send(WorkerCommand::Continue).map_err(|_| {
        verification_error(
            FIRST_VCPU_ID,
            "guest IPI worker resume channel",
            "worker exited before resume command",
        )
    })?;
    let second = worker.join().map_err(|_| {
        verification_error(
            SECOND_VCPU_ID,
            "guest IPI worker join",
            "second-vCPU worker panicked",
        )
    })??;

    let first_mainline = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'M',
        "guest IPI first-vCPU target isolation",
    )?;
    let first_completion = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'D',
        "guest IPI first-vCPU completion",
    )?;
    let first_completion_rflags = first_vcpu.registers()?.rflags;
    require_interrupt_enabled_flags(
        FIRST_VCPU_ID,
        "guest IPI first-vCPU completion state",
        first_completion_rflags,
    )?;
    let first_proof = first_port_io.debug_output().unwrap_or(&[]).to_vec();
    if first_proof.as_slice() != FIRST_PROOF {
        return Err(verification_error(
            FIRST_VCPU_ID,
            "guest IPI first-vCPU proof",
            format!("expected {:?}, got {first_proof:?}", FIRST_PROOF),
        ));
    }

    Ok(TwoVcpuGuestIpiResult {
        first_io_exits: vec![first_barrier, first_send, first_mainline, first_completion],
        second_io_exits: second.io_exits,
        first_proof,
        second_proof: second.proof,
        first_barrier_rflags,
        first_send_rflags,
        first_completion_rflags,
        second_ready_rflags: second.ready_rflags,
        second_completion_rflags: second.completion_rflags,
        second_mp_state,
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

fn join_worker_failure(detail: &'static str) -> Error {
    verification_error(SECOND_VCPU_ID, "guest IPI worker readiness", detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_icr_program_targets_apic_one_with_fixed_vector_52() {
        assert_eq!(LAPIC_VIRTUAL_PAGE, 0x50_0000);
        assert_eq!(LAPIC_GPA, 0xfee0_0000);
        assert_eq!(TARGET_APIC_ID, SECOND_VCPU_ID.get() as u8);
        assert_eq!(TARGET_VECTOR, 0x52);
        assert_eq!(ICR_HIGH_VALUE, 0x0100_0000);
        assert_eq!(ICR_LOW_VALUE, 0x52);
        assert_eq!(
            &FIRST_GUEST_BYTES[16..26],
            &[0xc7, 0x83, 0x10, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]
        );
        assert_eq!(
            &FIRST_GUEST_BYTES[26..36],
            &[0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x52, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn handler_acknowledges_local_apic_before_iretq() {
        assert_eq!(
            &HANDLER_BYTES[4..14],
            &[0xc7, 0x83, 0xb0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(&HANDLER_BYTES[14..], &[0x48, 0xcf]);
    }

    #[test]
    fn proofs_make_target_isolation_and_handler_order_observable() {
        assert_eq!(FIRST_PROOF, b"0SMD");
        assert_eq!(SECOND_PROOF, b"RI1D");
        assert_eq!(&SECOND_GUEST_BYTES[11..15], &[0xb0, b'R', 0xe6, 0xe9]);
        assert_eq!(&SECOND_GUEST_BYTES[15..17], &[0xfb, 0x90]);
    }
}
