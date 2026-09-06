use super::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::error::{Error, HostEnvironmentError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LongModeBootLayout, LONG_MODE_IDENTITY_MAP_SIZE};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;
use std::sync::mpsc;

const RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const BSP_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
const AP_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_1000);
const BSP_STACK: u64 = 0x1f_f000;
const AP_STACK: u64 = 0x1e_f000;
const BSP_EXIT_BUDGET: u32 = 5;
const AP_REMAINING_EXIT_BUDGET: u32 = 3;
const MAILBOX_EXTENT: usize = 0x19;

pub const BSP_VCPU_ID: VcpuId = VcpuId::BOOT;
pub const AP_VCPU_ID: VcpuId = VcpuId::new(1);
pub const WORK_MAILBOX: GuestPhysAddr = GuestPhysAddr::new(0x9000);
pub const WORK_COMMAND_OFFSET: usize = 0x08;
pub const WORK_RESULT_OFFSET: usize = 0x10;
pub const WORK_ACK_OFFSET: usize = 0x18;
pub const WORK_PAYLOAD: u8 = 0x21;
pub const WORK_RESULT: u8 = 0x42;
pub const BSP_WORK_PROOF: &[u8; 4] = b"BCVD";
pub const AP_WORK_PROOF: &[u8; 3] = b"RPD";
pub const BSP_TERMINAL_RIP: u64 = 0x1_0043;
pub const AP_TERMINAL_RIP: u64 = 0x1_103c;

// Assembled as 64-bit code at VMA 0x10000. Payload publication happens-before command
// publication through the implicitly locked memory XCHG. The acknowledgement is consumed with
// another memory XCHG before the BSP reads and validates the result.
#[rustfmt::skip]
const BSP_GUEST_BYTES: [u8; 72] = [
    0x48, 0xc7, 0xc3, 0x00, 0x90, 0x00, 0x00,
    0xb0, 0x42, 0xe6, 0xe9,
    0xc6, 0x03, 0x21,
    0xb0, 0x01, 0x86, 0x43, 0x08,
    0xb0, 0x43, 0xe6, 0xe9,
    0xb9, 0x00, 0x00, 0x00, 0x08,
    0x80, 0x7b, 0x18, 0x01, 0x74, 0x08,
    0xf3, 0x90, 0xff, 0xc9, 0x75, 0xf4, 0xeb, 0x19,
    0x31, 0xc0, 0x86, 0x43, 0x18, 0x3c, 0x01, 0x75, 0x10,
    0x8a, 0x43, 0x10, 0x3c, 0x42, 0x75, 0x09,
    0xb0, 0x56, 0xe6, 0xe9,
    0xb0, 0x44, 0xe6, 0xe9,
    0xf4,
    0xb0, 0x46, 0xe6, 0xe9, 0xf4,
];

// Assembled as 64-bit code at VMA 0x11000. The AP observes the command, claims it with an
// implicitly locked XCHG, doubles the payload, stores the result, then publishes the ack with a
// second memory XCHG. The bounded poll loop emits F instead of spinning forever on failure.
#[rustfmt::skip]
const AP_GUEST_BYTES: [u8; 65] = [
    0x48, 0xc7, 0xc3, 0x00, 0x90, 0x00, 0x00,
    0xb0, 0x52, 0xe6, 0xe9,
    0xb9, 0x00, 0x00, 0x00, 0x08,
    0x80, 0x7b, 0x08, 0x01, 0x74, 0x08,
    0xf3, 0x90, 0xff, 0xc9, 0x75, 0xf4, 0xeb, 0x1e,
    0x31, 0xc0, 0x86, 0x43, 0x08, 0x3c, 0x01, 0x75, 0x15,
    0x8a, 0x03, 0x00, 0xc0, 0x88, 0x43, 0x10,
    0xb0, 0x01, 0x86, 0x43, 0x18,
    0xb0, 0x50, 0xe6, 0xe9,
    0xb0, 0x44, 0xe6, 0xe9,
    0xf4,
    0xb0, 0x46, 0xe6, 0xe9, 0xf4,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkMailboxSnapshot {
    payload: u8,
    command: u8,
    result: u8,
    ack: u8,
}

impl WorkMailboxSnapshot {
    #[must_use]
    pub const fn payload(self) -> u8 {
        self.payload
    }

    #[must_use]
    pub const fn command(self) -> u8 {
        self.command
    }

    #[must_use]
    pub const fn result(self) -> u8 {
        self.result
    }

    #[must_use]
    pub const fn ack(self) -> u8 {
        self.ack
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuWorkDispatchResult {
    bsp_io_exits: Vec<PortIoExit>,
    ap_io_exits: Vec<PortIoExit>,
    bsp_proof: Vec<u8>,
    ap_proof: Vec<u8>,
    mailbox: WorkMailboxSnapshot,
    bsp_report: VmExitReport,
    ap_report: VmExitReport,
}

impl TwoVcpuWorkDispatchResult {
    #[must_use]
    pub fn bsp_io_exits(&self) -> &[PortIoExit] {
        &self.bsp_io_exits
    }

    #[must_use]
    pub fn ap_io_exits(&self) -> &[PortIoExit] {
        &self.ap_io_exits
    }

    #[must_use]
    pub fn bsp_proof(&self) -> &[u8] {
        &self.bsp_proof
    }

    #[must_use]
    pub fn ap_proof(&self) -> &[u8] {
        &self.ap_proof
    }

    #[must_use]
    pub const fn mailbox(&self) -> WorkMailboxSnapshot {
        self.mailbox
    }

    #[must_use]
    pub const fn bsp_report(&self) -> VmExitReport {
        self.bsp_report
    }

    #[must_use]
    pub const fn ap_report(&self) -> VmExitReport {
        self.ap_report
    }
}

#[derive(Debug)]
struct ApWorkerResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
}

pub fn run_two_vcpu_work_dispatch() -> Result<TwoVcpuWorkDispatchResult, Error> {
    let bsp_image = FlatGuestImage::new(BSP_ENTRY, BSP_ENTRY, &BSP_GUEST_BYTES)?;
    let ap_image = FlatGuestImage::new(AP_ENTRY, AP_ENTRY, &AP_GUEST_BYTES)?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(RAM_BASE, LONG_MODE_IDENTITY_MAP_SIZE)?;
    let bsp_layout = LongModeBootLayout::new(memory.region(), bsp_image.entry(), BSP_STACK)
        .expect("fixed BSP work-dispatch long-mode layout remains valid");
    let ap_layout = LongModeBootLayout::new(memory.region(), ap_image.entry(), AP_STACK)
        .expect("fixed AP work-dispatch long-mode layout remains valid");
    bsp_layout.install_page_tables(&mut memory)?;
    bsp_image.load(&mut memory)?;
    ap_image.load(&mut memory)?;
    memory.write(WORK_MAILBOX, &[0_u8; MAILBOX_EXTENT])?;
    vm.register_guest_memory(memory)?;

    let mut bsp_vcpu = vm.create_vcpu(BSP_VCPU_ID)?;
    let mut ap_vcpu = vm.create_vcpu(AP_VCPU_ID)?;
    bsp_vcpu.initialize_long_mode(&bsp_layout)?;
    ap_vcpu.initialize_long_mode(&ap_layout)?;

    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let worker = std::thread::spawn(move || -> Result<ApWorkerResult, Error> {
        let mut port_io = PortIoBus::with_debug_port();
        let ready = run_expected_debug_output(
            &mut ap_vcpu,
            &mut port_io,
            b'R',
            "AP work-dispatch readiness barrier",
        )?;
        ready_tx.send(()).map_err(|_| {
            verification_error(
                AP_VCPU_ID,
                "AP work-dispatch readiness channel",
                "BSP thread dropped readiness receiver",
            )
        })?;

        let execution = run_vcpu_until_stopped(
            &mut ap_vcpu,
            &mut port_io,
            AP_REMAINING_EXIT_BUDGET,
        )?;
        let mut io_exits = vec![ready];
        io_exits.extend_from_slice(execution.io_exits());
        Ok(ApWorkerResult {
            io_exits,
            proof: port_io.debug_output().unwrap_or(&[]).to_vec(),
            report: execution.report(),
        })
    });

    ready_rx.recv().map_err(|_| {
        verification_error(
            BSP_VCPU_ID,
            "BSP work-dispatch readiness wait",
            "AP worker exited before publishing readiness",
        )
    })?;

    let mut bsp_port_io = PortIoBus::with_debug_port();
    let bsp_execution = run_vcpu_until_stopped(
        &mut bsp_vcpu,
        &mut bsp_port_io,
        BSP_EXIT_BUDGET,
    )?;
    let bsp_proof = bsp_port_io.debug_output().unwrap_or(&[]).to_vec();

    let ap = worker.join().map_err(|_| {
        verification_error(
            AP_VCPU_ID,
            "AP work-dispatch worker join",
            "AP worker panicked",
        )
    })??;

    let mut bytes = [0_u8; MAILBOX_EXTENT];
    vm.guest_memory()
        .expect("registered work-dispatch guest memory remains VM-owned")
        .read(WORK_MAILBOX, &mut bytes)?;
    let mailbox = WorkMailboxSnapshot {
        payload: bytes[0],
        command: bytes[WORK_COMMAND_OFFSET],
        result: bytes[WORK_RESULT_OFFSET],
        ack: bytes[WORK_ACK_OFFSET],
    };

    if bsp_proof.as_slice() != BSP_WORK_PROOF {
        return Err(verification_error(
            BSP_VCPU_ID,
            "BSP work-dispatch proof",
            format!("expected {BSP_WORK_PROOF:?}, got {bsp_proof:?}"),
        ));
    }
    if ap.proof.as_slice() != AP_WORK_PROOF {
        return Err(verification_error(
            AP_VCPU_ID,
            "AP work-dispatch proof",
            format!("expected {AP_WORK_PROOF:?}, got {:?}", ap.proof),
        ));
    }
    if mailbox
        != (WorkMailboxSnapshot {
            payload: WORK_PAYLOAD,
            command: 0,
            result: WORK_RESULT,
            ack: 0,
        })
    {
        return Err(verification_error(
            BSP_VCPU_ID,
            "work-dispatch mailbox completion",
            format!("unexpected final mailbox state: {mailbox:?}"),
        ));
    }

    Ok(TwoVcpuWorkDispatchResult {
        bsp_io_exits: bsp_execution.io_exits().to_vec(),
        ap_io_exits: ap.io_exits,
        bsp_proof,
        ap_proof: ap.proof,
        mailbox,
        bsp_report: bsp_execution.report(),
        ap_report: ap.report,
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
    let io = vcpu.port_io_exit()?;
    if io.direction() != PortIoDirection::Out
        || io.port() != DEBUG_PORT
        || io.size() != 1
        || io.count() != 1
        || io.output_data() != [expected]
    {
        return Err(verification_error(
            vcpu.id(),
            stage,
            format!("unexpected debug output exit: {io:?}; expected byte {expected:#x}"),
        ));
    }
    if port_io.dispatch(&io)? != PortIoService::Output {
        return Err(verification_error(
            vcpu.id(),
            stage,
            "debug output unexpectedly requested an input response",
        ));
    }
    Ok(io)
}

fn verification_error(id: VcpuId, operation: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: id.get(),
        operation,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_programs_use_locked_mailbox_transitions_and_bounded_failure_paths() {
        assert_eq!(WORK_MAILBOX.get(), 0x9000);
        assert_eq!(WORK_COMMAND_OFFSET, 0x08);
        assert_eq!(WORK_RESULT_OFFSET, 0x10);
        assert_eq!(WORK_ACK_OFFSET, 0x18);
        assert_eq!(BSP_GUEST_BYTES.len(), 72);
        assert_eq!(AP_GUEST_BYTES.len(), 65);
        assert_eq!(BSP_TERMINAL_RIP, BSP_ENTRY.get() + 0x43);
        assert_eq!(AP_TERMINAL_RIP, AP_ENTRY.get() + 0x3c);
        assert!(BSP_GUEST_BYTES
            .windows(3)
            .any(|window| window == [0x86, 0x43, 0x08]));
        assert!(BSP_GUEST_BYTES
            .windows(3)
            .any(|window| window == [0x86, 0x43, 0x18]));
        assert!(AP_GUEST_BYTES
            .windows(3)
            .any(|window| window == [0x86, 0x43, 0x08]));
        assert!(AP_GUEST_BYTES
            .windows(3)
            .any(|window| window == [0x86, 0x43, 0x18]));
        assert_eq!(&BSP_GUEST_BYTES[67..72], &[0xb0, b'F', 0xe6, 0xe9, 0xf4]);
        assert_eq!(&AP_GUEST_BYTES[60..65], &[0xb0, b'F', 0xe6, 0xe9, 0xf4]);
        assert_eq!(BSP_WORK_PROOF, b"BCVD");
        assert_eq!(AP_WORK_PROOF, b"RPD");
    }
}
