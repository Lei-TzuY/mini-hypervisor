use crate::interrupt::{
    LongModeInterruptLayout, LONG_MODE_INTERRUPT_HANDLER, LONG_MODE_INTERRUPT_STACK_POINTER,
    LONG_MODE_INTERRUPT_VECTOR, X86_RFLAGS_INTERRUPT_ENABLE,
};
use crate::kvm::sys::{KvmLapicState, MasterPicStateSnapshot};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LongModeBootLayout, LONG_MODE_IDENTITY_MAP_SIZE};
use crate::portio::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::vcpu::{PortIoDirection, PortIoExit, VcpuExit};

pub const CONTROLLER_CHECKPOINT_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x10000);
pub const CONTROLLER_CHECKPOINT_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x30000);
pub const CONTROLLER_CHECKPOINT_CAPTURE_RIP: u64 = 0x10032;
pub const CONTROLLER_CHECKPOINT_PROOF: &[u8; 4] = b"AIMD";
pub const CONTROLLER_CHECKPOINT_MARKER: u8 = b'A';
const CONTROLLER_CHECKPOINT_CAPTURE_ARM_BYTE: u8 = b'C';
const CONTROLLER_CHECKPOINT_CORRUPT_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x12000);
const CONTROLLER_CHECKPOINT_CORRUPT_STACK: u64 = 0x1fdff8;
const APIC_SPIV_OFFSET: usize = 0x0f0;
const APIC_LVT0_OFFSET: usize = 0x350;
const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_DELIVERY_MODE_EXTINT: u32 = 0x700;

const CONTROLLER_CHECKPOINT_GUEST_BYTES: [u8; 70] = [
    0xfa,
    0xb0, 0x11, 0xe6, 0x20, 0xe6, 0xa0,
    0xb0, 0x40, 0xe6, 0x21,
    0xb0, 0x48, 0xe6, 0xa1,
    0xb0, 0x04, 0xe6, 0x21,
    0xb0, 0x02, 0xe6, 0xa1,
    0xb0, 0x01, 0xe6, 0x21, 0xe6, 0xa1,
    0xb0, 0xfe, 0xe6, 0x21,
    0xb0, 0xff, 0xe6, 0xa1,
    0xc6, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00, CONTROLLER_CHECKPOINT_MARKER,
    0xb0, CONTROLLER_CHECKPOINT_CAPTURE_ARM_BYTE, 0xe6, 0xe9,
    0x90,
    0xfb,
    0x90,
    0x8a, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00,
    0xe6, 0xe9,
    0xb0, b'M', 0xe6, 0xe9,
    0xb0, b'D', 0xe6, 0xe9,
    0xf4,
];

const CONTROLLER_CHECKPOINT_HANDLER_BYTES: [u8; 10] = [
    0xb0, b'I', 0xe6, 0xe9,
    0xb0, 0x20, 0xe6, 0x20,
    0x48, 0xcf,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControllerCheckpointCapture {
    rip: u64,
    rflags: u64,
}

impl ControllerCheckpointCapture {
    #[must_use]
    pub const fn rip(self) -> u64 {
        self.rip
    }

    #[must_use]
    pub const fn rflags(self) -> u64 {
        self.rflags
    }
}

impl std::fmt::Display for ControllerCheckpointCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "KVM_EXIT_DEBUG: rip={:#x}, rflags={:#x}",
            self.rip, self.rflags
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedControllerCheckpoint {
    guest: BoundedVcpuPageSetCheckpoint,
    master_pic: MasterPicStateSnapshot,
    lapic: KvmLapicState,
}

impl BoundedControllerCheckpoint {
    pub fn capture(
        vcpu: &Vcpu,
        vm: &crate::kvm::Vm,
        msr_policy: &GuestMsrAccessPolicy,
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let memory = vm.guest_memory().ok_or_else(|| {
            page_set_error("controller checkpoint capture", "VM has no registered guest memory")
        })?;
        let guest = BoundedVcpuPageSetCheckpoint::capture(vcpu, msr_policy, memory, page_addresses)?;
        let master_pic = vm.capture_master_pic_state()?;
        let lapic = vcpu.capture_lapic_checkpoint_state()?;
        Ok(Self {
            guest,
            master_pic,
            lapic,
        })
    }

    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPage] {
        self.guest.pages()
    }

    #[must_use]
    pub(crate) const fn master_pic(&self) -> &MasterPicStateSnapshot {
        &self.master_pic
    }

    #[must_use]
    pub(crate) const fn lapic(&self) -> &KvmLapicState {
        &self.lapic
    }

    pub fn verify(
        &self,
        vcpu: &Vcpu,
        vm: &crate::kvm::Vm,
    ) -> Result<BoundedControllerCheckpointComparison, Error> {
        let memory = vm.guest_memory().ok_or_else(|| {
            page_set_error(
                "controller checkpoint verification",
                "VM has no registered guest memory",
            )
        })?;
        let guest = self.guest.verify(vcpu, memory)?;
        let master_pic = vm.capture_master_pic_state()? == self.master_pic;
        let lapic = vcpu.capture_lapic_checkpoint_state()? == self.lapic;
        Ok(BoundedControllerCheckpointComparison {
            guest,
            master_pic,
            lapic,
        })
    }

    pub fn restore_and_verify(
        &self,
        vcpu: &Vcpu,
        vm: &mut crate::kvm::Vm,
    ) -> Result<BoundedControllerCheckpointComparison, Error> {
        let guest = {
            let memory = vm.guest_memory_mut().ok_or_else(|| {
                page_set_error(
                    "controller checkpoint restore",
                    "VM has no registered guest memory",
                )
            })?;
            self.guest.restore_and_verify(vcpu, memory)?
        };
        if !guest.is_exact_match() {
            return Err(page_set_error(
                "controller checkpoint guest restore verification",
                "page/VCPU state was not exact after bounded restore; controller restore was not attempted",
            ));
        }

        restore_controller_components_with(
            || vm.restore_master_pic_state(&self.master_pic),
            || vcpu.restore_lapic_checkpoint_state(&self.lapic),
        )?;

        self.verify(vcpu, vm)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedControllerCheckpointComparison {
    guest: BoundedPageSetCheckpointComparison,
    master_pic: bool,
    lapic: bool,
}

impl BoundedControllerCheckpointComparison {
    #[must_use]
    pub const fn guest(&self) -> &BoundedPageSetCheckpointComparison {
        &self.guest
    }

    #[must_use]
    pub fn page_exact(&self, address: GuestPhysAddr) -> Option<bool> {
        self.guest.page_exact(address)
    }

    #[must_use]
    pub fn vcpu_exact(&self) -> bool {
        self.guest.vcpu().is_exact_match()
    }

    #[must_use]
    pub const fn master_pic_exact(&self) -> bool {
        self.master_pic
    }

    #[must_use]
    pub const fn lapic_exact(&self) -> bool {
        self.lapic
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.guest.is_exact_match() && self.master_pic && self.lapic
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerCheckpointGuestResult {
    capture: ControllerCheckpointCapture,
    corruption: BoundedControllerCheckpointComparison,
    restored: BoundedControllerCheckpointComparison,
    captured_pic_imr: u8,
    captured_lapic_spiv: u32,
    captured_lapic_lint0: u32,
    armed_rflags: u64,
    completion_rflags: u64,
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
}

impl ControllerCheckpointGuestResult {
    #[must_use]
    pub const fn capture(&self) -> ControllerCheckpointCapture {
        self.capture
    }

    #[must_use]
    pub const fn corruption(&self) -> &BoundedControllerCheckpointComparison {
        &self.corruption
    }

    #[must_use]
    pub const fn restored(&self) -> &BoundedControllerCheckpointComparison {
        &self.restored
    }

    #[must_use]
    pub const fn captured_pic_imr(&self) -> u8 {
        self.captured_pic_imr
    }

    #[must_use]
    pub const fn captured_lapic_spiv(&self) -> u32 {
        self.captured_lapic_spiv
    }

    #[must_use]
    pub const fn captured_lapic_lint0(&self) -> u32 {
        self.captured_lapic_lint0
    }

    #[must_use]
    pub const fn armed_rflags(&self) -> u64 {
        self.armed_rflags
    }

    #[must_use]
    pub const fn completion_rflags(&self) -> u64 {
        self.completion_rflags
    }

    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
}

pub fn run_controller_checkpoint_guest() -> Result<ControllerCheckpointGuestResult, Error> {
    let guest = FlatGuestImage::new(
        CONTROLLER_CHECKPOINT_ENTRY,
        CONTROLLER_CHECKPOINT_ENTRY,
        &CONTROLLER_CHECKPOINT_GUEST_BYTES,
    )?;
    let handler = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_HANDLER,
        LONG_MODE_INTERRUPT_HANDLER,
        &CONTROLLER_CHECKPOINT_HANDLER_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModeInterruptLayout::new(
        memory.region(),
        guest.entry(),
        LONG_MODE_INTERRUPT_STACK_POINTER,
        LONG_MODE_INTERRUPT_VECTOR,
        handler.entry(),
    )
    .expect("fixed controller checkpoint interrupt layout remains valid");
    let corrupt_layout = LongModeBootLayout::new(
        memory.region(),
        CONTROLLER_CHECKPOINT_CORRUPT_ENTRY,
        CONTROLLER_CHECKPOINT_CORRUPT_STACK,
    )
    .expect("fixed controller checkpoint corruption layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(&layout)?;
    let _ = vcpu.configure_legacy_pic_extint()?;
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty controller checkpoint MSR policy is valid by construction");

    let capture = controller_run_to_quiescent_debug(&mut vcpu)?;
    let checkpoint = BoundedControllerCheckpoint::capture(
        &vcpu,
        &vm,
        &msr_policy,
        &[CONTROLLER_CHECKPOINT_PAGE],
    )?;
    controller_require_capture_contract(&checkpoint)?;

    vm.guest_memory_mut()
        .expect("registered controller checkpoint memory remains VM-owned")
        .write(
            CONTROLLER_CHECKPOINT_PAGE,
            &vec![0xa5; LONG_MODE_PAGE_SIZE as usize],
        )?;
    vcpu.initialize_long_mode(&corrupt_layout)?;
    let corrupt_pic = checkpoint
        .master_pic()
        .with_imr(checkpoint.master_pic().imr() ^ 0x01);
    vm.restore_master_pic_state(&corrupt_pic)?;
    let mut corrupt_lapic = checkpoint.lapic().clone();
    let lint0 = controller_read_lapic_register(&corrupt_lapic, APIC_LVT0_OFFSET);
    controller_write_lapic_register(
        &mut corrupt_lapic,
        APIC_LVT0_OFFSET,
        lint0 | APIC_LVT_MASKED,
    );
    vcpu.restore_lapic_checkpoint_state(&corrupt_lapic)?;

    let corruption = checkpoint.verify(&vcpu, &vm)?;
    controller_require_full_mismatch(&corruption)?;

    let restored = checkpoint.restore_and_verify(&vcpu, &mut vm)?;
    if !restored.is_exact_match() {
        return Err(page_set_error(
            "controller checkpoint restore verification",
            format!(
                "restore mismatch: page={:?} vcpu={} master-pic={} lapic={}",
                restored.page_exact(CONTROLLER_CHECKPOINT_PAGE),
                restored.vcpu_exact(),
                restored.master_pic_exact(),
                restored.lapic_exact()
            ),
        ));
    }

    let mut port_io = PortIoBus::with_debug_port();
    let armed_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        CONTROLLER_CHECKPOINT_MARKER,
        "controller checkpoint restored-page armed barrier",
    )?;
    let armed = vcpu.registers()?;
    controller_require_interrupt_enabled("controller checkpoint armed state", armed.rflags)?;

    vm.pulse_gsi_edge(0)?;
    let handler_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'I',
        "controller checkpoint interrupt handler",
    )?;
    let resumed_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'M',
        "controller checkpoint resumed main",
    )?;
    let completion_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'D',
        "controller checkpoint completion barrier",
    )?;
    let completion = vcpu.registers()?;
    controller_require_interrupt_enabled("controller checkpoint completion state", completion.rflags)?;

    let io_exits = vec![armed_io, handler_io, resumed_io, completion_io];
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != CONTROLLER_CHECKPOINT_PROOF
        || io_exits.len() != CONTROLLER_CHECKPOINT_PROOF.len()
    {
        return Err(page_set_error(
            "controller checkpoint executable proof",
            format!(
                "expected {:?} across {} exits, got {:?} across {} exits",
                CONTROLLER_CHECKPOINT_PROOF,
                CONTROLLER_CHECKPOINT_PROOF.len(),
                proof,
                io_exits.len()
            ),
        ));
    }

    Ok(ControllerCheckpointGuestResult {
        capture,
        corruption,
        restored,
        captured_pic_imr: checkpoint.master_pic().imr(),
        captured_lapic_spiv: controller_read_lapic_register(checkpoint.lapic(), APIC_SPIV_OFFSET),
        captured_lapic_lint0: controller_read_lapic_register(checkpoint.lapic(), APIC_LVT0_OFFSET),
        armed_rflags: armed.rflags,
        completion_rflags: completion.rflags,
        io_exits,
        proof,
    })
}

fn restore_controller_components_with<E, P, L>(mut restore_pic: P, mut restore_lapic: L) -> Result<(), E>
where
    P: FnMut() -> Result<(), E>,
    L: FnMut() -> Result<(), E>,
{
    restore_pic()?;
    restore_lapic()?;
    Ok(())
}

fn controller_run_to_quiescent_debug(
    vcpu: &mut Vcpu,
) -> Result<ControllerCheckpointCapture, Error> {
    let mut capture_io = PortIoBus::with_debug_port();
    controller_run_expected_debug_output(
        vcpu,
        &mut capture_io,
        CONTROLLER_CHECKPOINT_CAPTURE_ARM_BYTE,
        "controller checkpoint capture arm barrier",
    )?;

    vcpu.set_guest_single_step(true)?;
    let exit_result = vcpu.run_once();
    let disable_result = vcpu.set_guest_single_step(false);
    let exit = match (exit_result, disable_result) {
        (Ok(exit), Ok(())) => exit,
        (Err(error), _) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
    };
    if exit != VcpuExit::Debug {
        return Err(page_set_error(
            "controller checkpoint quiescent debug boundary",
            format!(
                "expected KVM_EXIT_DEBUG reason {}, got {}",
                VcpuExit::Debug.reason(),
                exit.reason()
            ),
        ));
    }

    let registers = vcpu.registers()?;
    if registers.rip != CONTROLLER_CHECKPOINT_CAPTURE_RIP
        || registers.rflags & 0x2 != 0x2
        || registers.rflags & X86_RFLAGS_INTERRUPT_ENABLE != 0
    {
        return Err(page_set_error(
            "controller checkpoint quiescent debug boundary",
            format!(
                "expected IF-clear debug boundary at rip={:#x}, got rip={:#x}, rflags={:#x}",
                CONTROLLER_CHECKPOINT_CAPTURE_RIP, registers.rip, registers.rflags
            ),
        ));
    }

    Ok(ControllerCheckpointCapture {
        rip: registers.rip,
        rflags: registers.rflags,
    })
}

fn controller_require_capture_contract(checkpoint: &BoundedControllerCheckpoint) -> Result<(), Error> {
    let marker = checkpoint
        .pages()
        .iter()
        .find(|page| page.address() == CONTROLLER_CHECKPOINT_PAGE)
        .and_then(|page| page.bytes().first())
        .copied();
    let spiv = controller_read_lapic_register(checkpoint.lapic(), APIC_SPIV_OFFSET);
    let lint0 = controller_read_lapic_register(checkpoint.lapic(), APIC_LVT0_OFFSET);
    if marker != Some(CONTROLLER_CHECKPOINT_MARKER)
        || checkpoint.master_pic().imr() != 0xfe
        || spiv & APIC_SPIV_SOFTWARE_ENABLE != APIC_SPIV_SOFTWARE_ENABLE
        || lint0 & APIC_LVT_DELIVERY_MODE_MASK != APIC_LVT_DELIVERY_MODE_EXTINT
        || lint0 & APIC_LVT_MASKED != 0
    {
        return Err(page_set_error(
            "controller checkpoint capture contract",
            format!(
                "expected marker A, PIC IMR 0xfe and enabled/unmasked ExtINT LAPIC; got marker={marker:?}, imr={:#x}, spiv={spiv:#x}, lint0={lint0:#x}",
                checkpoint.master_pic().imr()
            ),
        ));
    }
    Ok(())
}

fn controller_require_full_mismatch(
    comparison: &BoundedControllerCheckpointComparison,
) -> Result<(), Error> {
    if comparison.page_exact(CONTROLLER_CHECKPOINT_PAGE) != Some(false)
        || comparison.vcpu_exact()
        || comparison.master_pic_exact()
        || comparison.lapic_exact()
    {
        return Err(page_set_error(
            "controller checkpoint corruption proof",
            format!(
                "expected page/vcpu/master-pic/lapic mismatches, got page={:?} vcpu={} master-pic={} lapic={}",
                comparison.page_exact(CONTROLLER_CHECKPOINT_PAGE),
                comparison.vcpu_exact(),
                comparison.master_pic_exact(),
                comparison.lapic_exact()
            ),
        ));
    }
    Ok(())
}

fn controller_run_expected_debug_output(
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    expected: u8,
    stage: &'static str,
) -> Result<PortIoExit, Error> {
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Io {
        return Err(page_set_error(
            stage,
            format!(
                "expected KVM_EXIT_IO reason {}, got {}",
                VcpuExit::Io.reason(),
                exit.reason()
            ),
        ));
    }
    let io_exit = vcpu.port_io_exit()?;
    if io_exit.direction() != PortIoDirection::Out
        || io_exit.size() != 1
        || io_exit.port() != DEBUG_PORT
        || io_exit.count() != 1
        || io_exit.output_data() != [expected]
    {
        return Err(page_set_error(stage, format!("unexpected debug-port exit {io_exit:?}")));
    }
    if port_io.dispatch(&io_exit)? != PortIoService::Output {
        return Err(page_set_error(stage, "debug output unexpectedly requested input"));
    }
    Ok(io_exit)
}

fn controller_require_interrupt_enabled(operation: &'static str, rflags: u64) -> Result<(), Error> {
    if rflags & 0x2 != 0x2 || rflags & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE {
        return Err(page_set_error(
            operation,
            format!("expected RFLAGS bit1 and IF set, got {rflags:#x}"),
        ));
    }
    Ok(())
}

fn controller_read_lapic_register(state: &KvmLapicState, offset: usize) -> u32 {
    u32::from_le_bytes(
        state.regs[offset..offset + 4]
            .try_into()
            .expect("fixed LAPIC register offset remains in 0x400-byte state"),
    )
}

fn controller_write_lapic_register(state: &mut KvmLapicState, offset: usize, value: u32) {
    state.regs[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod controller_checkpoint_tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn controller_restore_orders_master_pic_before_lapic_and_stops_on_failure() {
        let sequence = RefCell::new(Vec::new());
        restore_controller_components_with(
            || {
                sequence.borrow_mut().push("pic");
                Ok::<_, &'static str>(())
            },
            || {
                sequence.borrow_mut().push("lapic");
                Ok::<_, &'static str>(())
            },
        )
        .unwrap();
        assert_eq!(&*sequence.borrow(), &["pic", "lapic"]);

        sequence.borrow_mut().clear();
        let error = restore_controller_components_with(
            || {
                sequence.borrow_mut().push("pic");
                Err::<(), _>("pic failure")
            },
            || {
                sequence.borrow_mut().push("lapic");
                Ok::<_, &'static str>(())
            },
        )
        .unwrap_err();
        assert_eq!(error, "pic failure");
        assert_eq!(&*sequence.borrow(), &["pic"]);
    }

    #[test]
    fn deterministic_guest_places_capture_and_resume_contract_exactly() {
        assert_eq!(CONTROLLER_CHECKPOINT_GUEST_BYTES.len(), 70);
        assert_eq!(
            &CONTROLLER_CHECKPOINT_GUEST_BYTES[45..49],
            &[0xb0, CONTROLLER_CHECKPOINT_CAPTURE_ARM_BYTE, 0xe6, 0xe9]
        );
        assert_eq!(CONTROLLER_CHECKPOINT_GUEST_BYTES[49], 0x90);
        assert_eq!(
            CONTROLLER_CHECKPOINT_CAPTURE_RIP,
            CONTROLLER_CHECKPOINT_ENTRY.get() + 50
        );
        assert_eq!(&CONTROLLER_CHECKPOINT_GUEST_BYTES[50..52], &[0xfb, 0x90]);
        assert_eq!(CONTROLLER_CHECKPOINT_GUEST_BYTES[69], 0xf4);
        assert_eq!(CONTROLLER_CHECKPOINT_HANDLER_BYTES.len(), 10);
        assert_eq!(CONTROLLER_CHECKPOINT_PROOF, b"AIMD");
    }
}
