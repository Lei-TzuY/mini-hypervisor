pub const FULL_CONTROLLER_CHECKPOINT_PROOF: &[u8; 6] = b"ASBJMD";
pub const FULL_CONTROLLER_SLAVE_GSI: u32 = 8;
pub const FULL_CONTROLLER_SLAVE_VECTOR: u8 = 0x48;
pub const FULL_CONTROLLER_IOAPIC_GSI: u32 = 16;
pub const FULL_CONTROLLER_IOAPIC_VECTOR: u8 = 0x50;
const FULL_CONTROLLER_IOAPIC_PIN8: usize = 8;
const FULL_CONTROLLER_IOAPIC_PIN16: usize = 16;
const FULL_CONTROLLER_IOAPIC_REDIR_MASKED: u64 = 1 << 16;
const FULL_CONTROLLER_IOAPIC_FIXED_ENTRY: u64 = FULL_CONTROLLER_IOAPIC_VECTOR as u64;
const FULL_CONTROLLER_IOAPIC_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x12000);

const FULL_CONTROLLER_GUEST_BYTES: [u8; 74] = [
    0xfa, // cli
    0xb0, 0x11, 0xe6, 0x20, 0xe6, 0xa0, // ICW1 master + slave
    0xb0, 0x40, 0xe6, 0x21, // master base 0x40
    0xb0, 0x48, 0xe6, 0xa1, // slave base 0x48
    0xb0, 0x04, 0xe6, 0x21, // master has slave on IRQ2
    0xb0, 0x02, 0xe6, 0xa1, // slave cascade identity 2
    0xb0, 0x01, 0xe6, 0x21, 0xe6, 0xa1, // 8086 mode
    0xb0, 0xfb, 0xe6, 0x21, // unmask only master cascade IRQ2
    0xb0, 0xfe, 0xe6, 0xa1, // unmask only slave IRQ0 / GSI8
    0xc6, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00, CONTROLLER_CHECKPOINT_MARKER,
    0xb0, CONTROLLER_CHECKPOINT_CAPTURE_ARM_BYTE, 0xe6, 0xe9,
    0x90, // single-step quiescent target
    0xfb, 0x90, // sti; nop
    0x8a, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00, // restored marker A
    0xe6, 0xe9, // A: host pulses GSI8
    0xb0, b'B', 0xe6, 0xe9, // B: slave-PIC interrupt returned; host pulses GSI16
    0xb0, b'M', 0xe6, 0xe9, // M: IOAPIC interrupt returned
    0xb0, b'D', 0xe6, 0xe9, // D: completion barrier
    0xf4,
];

const FULL_CONTROLLER_SLAVE_HANDLER_BYTES: [u8; 14] = [
    0xb0, b'S', 0xe6, 0xe9, // slave-PIC handler identity
    0xb0, 0x20, 0xe6, 0xa0, // EOI slave PIC
    0xb0, 0x20, 0xe6, 0x20, // EOI master cascade
    0x48, 0xcf, // iretq
];

const FULL_CONTROLLER_IOAPIC_HANDLER_BYTES: [u8; 6] = [
    0xb0, b'J', 0xe6, 0xe9, // IOAPIC-only GSI16 handler identity
    0x48, 0xcf, // iretq; one-shot edge proof does not claim LAPIC EOI lifecycle
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedFullControllerCheckpoint {
    base: BoundedControllerCheckpoint,
    slave_pic: crate::kvm::sys::SlavePicStateSnapshot,
    ioapic: crate::kvm::sys::IoapicStateSnapshot,
}

impl BoundedFullControllerCheckpoint {
    pub fn capture(
        vcpu: &Vcpu,
        vm: &crate::kvm::Vm,
        msr_policy: &GuestMsrAccessPolicy,
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let base = BoundedControllerCheckpoint::capture(vcpu, vm, msr_policy, page_addresses)?;
        let slave_pic = vm.capture_slave_pic_state()?;
        let ioapic = vm.capture_ioapic_state()?;
        if ioapic.irr() != 0 {
            return Err(page_set_error(
                "full controller checkpoint IOAPIC quiescence",
                format!(
                    "pending IOAPIC IRR {:#x} is outside the bounded checkpoint contract",
                    ioapic.irr()
                ),
            ));
        }
        Ok(Self {
            base,
            slave_pic,
            ioapic,
        })
    }

    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPage] {
        self.base.pages()
    }

    #[must_use]
    pub(crate) const fn master_pic(&self) -> &crate::kvm::sys::MasterPicStateSnapshot {
        self.base.master_pic()
    }

    #[must_use]
    pub(crate) const fn slave_pic(&self) -> &crate::kvm::sys::SlavePicStateSnapshot {
        &self.slave_pic
    }

    #[must_use]
    pub(crate) const fn ioapic(&self) -> &crate::kvm::sys::IoapicStateSnapshot {
        &self.ioapic
    }

    #[must_use]
    pub(crate) const fn lapic(&self) -> &crate::kvm::sys::KvmLapicState {
        self.base.lapic()
    }

    pub fn verify(
        &self,
        vcpu: &Vcpu,
        vm: &crate::kvm::Vm,
    ) -> Result<BoundedFullControllerCheckpointComparison, Error> {
        let base = self.base.verify(vcpu, vm)?;
        let slave_pic = vm.capture_slave_pic_state()? == self.slave_pic;
        let ioapic = vm.capture_ioapic_state()? == self.ioapic;
        Ok(BoundedFullControllerCheckpointComparison {
            base,
            slave_pic,
            ioapic,
        })
    }

    pub fn restore_and_verify(
        &self,
        vcpu: &Vcpu,
        vm: &mut crate::kvm::Vm,
    ) -> Result<BoundedFullControllerCheckpointComparison, Error> {
        let guest = {
            let memory = vm.guest_memory_mut().ok_or_else(|| {
                page_set_error(
                    "full controller checkpoint restore",
                    "VM has no registered guest memory",
                )
            })?;
            self.base.guest.restore_and_verify(vcpu, memory)?
        };
        if !guest.is_exact_match() {
            return Err(page_set_error(
                "full controller checkpoint guest restore verification",
                "page/VCPU state was not exact after bounded restore; controller restore was not attempted",
            ));
        }

        restore_full_controller_components_with(
            || vm.restore_master_pic_state(self.base.master_pic()),
            || vm.restore_slave_pic_state(&self.slave_pic),
            || vm.restore_ioapic_state(&self.ioapic),
            || vcpu.restore_lapic_checkpoint_state(self.base.lapic()),
        )?;

        self.verify(vcpu, vm)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedFullControllerCheckpointComparison {
    base: BoundedControllerCheckpointComparison,
    slave_pic: bool,
    ioapic: bool,
}

impl BoundedFullControllerCheckpointComparison {
    #[must_use]
    pub fn page_exact(&self, address: GuestPhysAddr) -> Option<bool> {
        self.base.page_exact(address)
    }

    #[must_use]
    pub fn vcpu_exact(&self) -> bool {
        self.base.vcpu_exact()
    }

    #[must_use]
    pub const fn master_pic_exact(&self) -> bool {
        self.base.master_pic_exact()
    }

    #[must_use]
    pub const fn slave_pic_exact(&self) -> bool {
        self.slave_pic
    }

    #[must_use]
    pub const fn ioapic_exact(&self) -> bool {
        self.ioapic
    }

    #[must_use]
    pub const fn lapic_exact(&self) -> bool {
        self.base.lapic_exact()
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.base.is_exact_match() && self.slave_pic && self.ioapic
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullControllerCheckpointGuestResult {
    capture: ControllerCheckpointCapture,
    corruption: BoundedFullControllerCheckpointComparison,
    restored: BoundedFullControllerCheckpointComparison,
    captured_master_pic_imr: u8,
    captured_slave_pic_imr: u8,
    captured_ioapic_base: u64,
    captured_ioapic_pin16: u64,
    captured_lapic_spiv: u32,
    captured_lapic_lint0: u32,
    slave_armed_rflags: u64,
    ioapic_armed_rflags: u64,
    completion_rflags: u64,
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
}

impl FullControllerCheckpointGuestResult {
    #[must_use]
    pub const fn capture(&self) -> ControllerCheckpointCapture {
        self.capture
    }

    #[must_use]
    pub const fn corruption(&self) -> &BoundedFullControllerCheckpointComparison {
        &self.corruption
    }

    #[must_use]
    pub const fn restored(&self) -> &BoundedFullControllerCheckpointComparison {
        &self.restored
    }

    #[must_use]
    pub const fn captured_master_pic_imr(&self) -> u8 {
        self.captured_master_pic_imr
    }

    #[must_use]
    pub const fn captured_slave_pic_imr(&self) -> u8 {
        self.captured_slave_pic_imr
    }

    #[must_use]
    pub const fn captured_ioapic_base(&self) -> u64 {
        self.captured_ioapic_base
    }

    #[must_use]
    pub const fn captured_ioapic_pin16(&self) -> u64 {
        self.captured_ioapic_pin16
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
    pub const fn slave_armed_rflags(&self) -> u64 {
        self.slave_armed_rflags
    }

    #[must_use]
    pub const fn ioapic_armed_rflags(&self) -> u64 {
        self.ioapic_armed_rflags
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

pub fn run_full_controller_checkpoint_guest() -> Result<FullControllerCheckpointGuestResult, Error> {
    let guest = crate::loader::FlatGuestImage::new(
        CONTROLLER_CHECKPOINT_ENTRY,
        CONTROLLER_CHECKPOINT_ENTRY,
        &FULL_CONTROLLER_GUEST_BYTES,
    )?;
    let slave_handler = crate::loader::FlatGuestImage::new(
        LONG_MODE_INTERRUPT_HANDLER,
        LONG_MODE_INTERRUPT_HANDLER,
        &FULL_CONTROLLER_SLAVE_HANDLER_BYTES,
    )?;
    let ioapic_handler = crate::loader::FlatGuestImage::new(
        FULL_CONTROLLER_IOAPIC_HANDLER,
        FULL_CONTROLLER_IOAPIC_HANDLER,
        &FULL_CONTROLLER_IOAPIC_HANDLER_BYTES,
    )?;

    let backend = crate::kvm::KvmBackend::open()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = crate::interrupt::LongModeInterruptLayout::with_gates(
        memory.region(),
        guest.entry(),
        LONG_MODE_INTERRUPT_STACK_POINTER,
        vec![
            crate::interrupt::LongModeInterruptGate::new(
                FULL_CONTROLLER_SLAVE_VECTOR,
                slave_handler.entry(),
            ),
            crate::interrupt::LongModeInterruptGate::new(
                FULL_CONTROLLER_IOAPIC_VECTOR,
                ioapic_handler.entry(),
            ),
        ],
    )
    .expect("fixed full-controller interrupt layout remains valid");
    let corrupt_layout = crate::long_mode::LongModeBootLayout::new(
        memory.region(),
        CONTROLLER_CHECKPOINT_CORRUPT_ENTRY,
        CONTROLLER_CHECKPOINT_CORRUPT_STACK,
    )
    .expect("fixed full-controller corruption layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    slave_handler.load(&mut memory)?;
    ioapic_handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(&layout)?;
    let _ = vcpu.configure_legacy_pic_extint()?;
    full_controller_configure_ioapic(&vm)?;
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty full-controller checkpoint MSR policy is valid by construction");

    let capture = controller_run_to_quiescent_debug(&mut vcpu)?;
    let checkpoint = BoundedFullControllerCheckpoint::capture(
        &vcpu,
        &vm,
        &msr_policy,
        &[CONTROLLER_CHECKPOINT_PAGE],
    )?;
    full_controller_require_capture_contract(&checkpoint)?;

    vm.guest_memory_mut()
        .expect("registered full-controller checkpoint memory remains VM-owned")
        .write(
            CONTROLLER_CHECKPOINT_PAGE,
            &vec![0xa5; LONG_MODE_PAGE_SIZE as usize],
        )?;
    vcpu.initialize_long_mode(&corrupt_layout)?;
    vm.restore_master_pic_state(
        &checkpoint
            .master_pic()
            .with_imr(checkpoint.master_pic().imr() ^ 0x01),
    )?;
    vm.restore_slave_pic_state(
        &checkpoint
            .slave_pic()
            .with_imr(checkpoint.slave_pic().imr() ^ 0x01),
    )?;
    let corrupt_ioapic = checkpoint
        .ioapic()
        .with_redirection_entry(
            FULL_CONTROLLER_IOAPIC_PIN16,
            checkpoint
                .ioapic()
                .redirection_entry(FULL_CONTROLLER_IOAPIC_PIN16)
                .expect("fixed IOAPIC pin remains in range")
                | FULL_CONTROLLER_IOAPIC_REDIR_MASKED,
        )
        .expect("fixed IOAPIC pin remains in range");
    vm.restore_ioapic_state(&corrupt_ioapic)?;
    let mut corrupt_lapic = checkpoint.lapic().clone();
    let lint0 = controller_read_lapic_register(&corrupt_lapic, APIC_LVT0_OFFSET);
    controller_write_lapic_register(
        &mut corrupt_lapic,
        APIC_LVT0_OFFSET,
        lint0 | APIC_LVT_MASKED,
    );
    vcpu.restore_lapic_checkpoint_state(&corrupt_lapic)?;

    let corruption = checkpoint.verify(&vcpu, &vm)?;
    full_controller_require_full_mismatch(&corruption)?;

    let restored = checkpoint.restore_and_verify(&vcpu, &mut vm)?;
    if !restored.is_exact_match() {
        return Err(page_set_error(
            "full controller checkpoint restore verification",
            format!(
                "restore mismatch: page={:?} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={}",
                restored.page_exact(CONTROLLER_CHECKPOINT_PAGE),
                restored.vcpu_exact(),
                restored.master_pic_exact(),
                restored.slave_pic_exact(),
                restored.ioapic_exact(),
                restored.lapic_exact()
            ),
        ));
    }

    let mut port_io = PortIoBus::with_debug_port();
    let marker_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        CONTROLLER_CHECKPOINT_MARKER,
        "full controller restored-page barrier",
    )?;
    let slave_armed = vcpu.registers()?;
    controller_require_interrupt_enabled("full controller slave-PIC armed state", slave_armed.rflags)?;

    vm.pulse_gsi_edge(FULL_CONTROLLER_SLAVE_GSI)?;
    let slave_handler_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'S',
        "full controller slave-PIC handler",
    )?;
    let bridge_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'B',
        "full controller post-slave barrier",
    )?;
    let ioapic_armed = vcpu.registers()?;
    controller_require_interrupt_enabled("full controller IOAPIC armed state", ioapic_armed.rflags)?;

    vm.pulse_gsi_edge(FULL_CONTROLLER_IOAPIC_GSI)?;
    let ioapic_handler_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'J',
        "full controller IOAPIC handler",
    )?;
    let resumed_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'M',
        "full controller resumed main",
    )?;
    let completion_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'D',
        "full controller completion barrier",
    )?;
    let completion = vcpu.registers()?;
    controller_require_interrupt_enabled("full controller completion state", completion.rflags)?;

    let io_exits = vec![
        marker_io,
        slave_handler_io,
        bridge_io,
        ioapic_handler_io,
        resumed_io,
        completion_io,
    ];
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != FULL_CONTROLLER_CHECKPOINT_PROOF
        || io_exits.len() != FULL_CONTROLLER_CHECKPOINT_PROOF.len()
    {
        return Err(page_set_error(
            "full controller executable proof",
            format!(
                "expected {:?} across {} exits, got {:?} across {} exits",
                FULL_CONTROLLER_CHECKPOINT_PROOF,
                FULL_CONTROLLER_CHECKPOINT_PROOF.len(),
                proof,
                io_exits.len()
            ),
        ));
    }

    Ok(FullControllerCheckpointGuestResult {
        capture,
        corruption,
        restored,
        captured_master_pic_imr: checkpoint.master_pic().imr(),
        captured_slave_pic_imr: checkpoint.slave_pic().imr(),
        captured_ioapic_base: checkpoint.ioapic().base_address(),
        captured_ioapic_pin16: checkpoint
            .ioapic()
            .redirection_entry(FULL_CONTROLLER_IOAPIC_PIN16)
            .expect("fixed IOAPIC pin remains in range"),
        captured_lapic_spiv: controller_read_lapic_register(checkpoint.lapic(), APIC_SPIV_OFFSET),
        captured_lapic_lint0: controller_read_lapic_register(checkpoint.lapic(), APIC_LVT0_OFFSET),
        slave_armed_rflags: slave_armed.rflags,
        ioapic_armed_rflags: ioapic_armed.rflags,
        completion_rflags: completion.rflags,
        io_exits,
        proof,
    })
}

fn full_controller_configure_ioapic(vm: &crate::kvm::Vm) -> Result<(), Error> {
    let snapshot = vm.capture_ioapic_state()?;
    if snapshot.irr() != 0 {
        return Err(page_set_error(
            "full controller initial IOAPIC quiescence",
            format!("expected zero IOAPIC IRR, got {:#x}", snapshot.irr()),
        ));
    }
    let pin8 = snapshot
        .redirection_entry(FULL_CONTROLLER_IOAPIC_PIN8)
        .expect("fixed IOAPIC pin remains in range")
        | FULL_CONTROLLER_IOAPIC_REDIR_MASKED;
    let configured = snapshot
        .with_redirection_entry(FULL_CONTROLLER_IOAPIC_PIN8, pin8)
        .and_then(|state| {
            state.with_redirection_entry(
                FULL_CONTROLLER_IOAPIC_PIN16,
                FULL_CONTROLLER_IOAPIC_FIXED_ENTRY,
            )
        })
        .expect("fixed IOAPIC pins remain in range");
    vm.restore_ioapic_state(&configured)?;
    let readback = vm.capture_ioapic_state()?;
    if readback != configured {
        return Err(page_set_error(
            "full controller IOAPIC configuration readback",
            "IOAPIC pin8 mask / pin16 fixed-vector configuration did not read back exactly",
        ));
    }
    Ok(())
}

fn full_controller_require_capture_contract(
    checkpoint: &BoundedFullControllerCheckpoint,
) -> Result<(), Error> {
    let marker = checkpoint
        .pages()
        .iter()
        .find(|page| page.address() == CONTROLLER_CHECKPOINT_PAGE)
        .and_then(|page| page.bytes().first())
        .copied();
    let spiv = controller_read_lapic_register(checkpoint.lapic(), APIC_SPIV_OFFSET);
    let lint0 = controller_read_lapic_register(checkpoint.lapic(), APIC_LVT0_OFFSET);
    let pin8 = checkpoint
        .ioapic()
        .redirection_entry(FULL_CONTROLLER_IOAPIC_PIN8)
        .expect("fixed IOAPIC pin remains in range");
    let pin16 = checkpoint
        .ioapic()
        .redirection_entry(FULL_CONTROLLER_IOAPIC_PIN16)
        .expect("fixed IOAPIC pin remains in range");
    if marker != Some(CONTROLLER_CHECKPOINT_MARKER)
        || checkpoint.master_pic().imr() != 0xfb
        || checkpoint.slave_pic().imr() != 0xfe
        || checkpoint.ioapic().irr() != 0
        || checkpoint.ioapic().pad() != 0
        || pin8 & FULL_CONTROLLER_IOAPIC_REDIR_MASKED == 0
        || pin16 != FULL_CONTROLLER_IOAPIC_FIXED_ENTRY
        || spiv & APIC_SPIV_SOFTWARE_ENABLE != APIC_SPIV_SOFTWARE_ENABLE
        || lint0 & APIC_LVT_DELIVERY_MODE_MASK != APIC_LVT_DELIVERY_MODE_EXTINT
        || lint0 & APIC_LVT_MASKED != 0
    {
        return Err(page_set_error(
            "full controller checkpoint capture contract",
            format!(
                "expected marker A, master/slave IMR 0xfb/0xfe, quiescent IOAPIC pin8 masked pin16={FULL_CONTROLLER_IOAPIC_FIXED_ENTRY:#x}, and enabled/unmasked ExtINT LAPIC; got marker={marker:?}, master={:#x}, slave={:#x}, irr={:#x}, ioapic-pad={:#x}, pin8={pin8:#x}, pin16={pin16:#x}, spiv={spiv:#x}, lint0={lint0:#x}",
                checkpoint.master_pic().imr(),
                checkpoint.slave_pic().imr(),
                checkpoint.ioapic().irr(),
                checkpoint.ioapic().pad(),
            ),
        ));
    }
    Ok(())
}

fn full_controller_require_full_mismatch(
    comparison: &BoundedFullControllerCheckpointComparison,
) -> Result<(), Error> {
    if comparison.page_exact(CONTROLLER_CHECKPOINT_PAGE) != Some(false)
        || comparison.vcpu_exact()
        || comparison.master_pic_exact()
        || comparison.slave_pic_exact()
        || comparison.ioapic_exact()
        || comparison.lapic_exact()
    {
        return Err(page_set_error(
            "full controller checkpoint corruption proof",
            format!(
                "expected page/vcpu/master-pic/slave-pic/ioapic/lapic mismatches, got page={:?} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={}",
                comparison.page_exact(CONTROLLER_CHECKPOINT_PAGE),
                comparison.vcpu_exact(),
                comparison.master_pic_exact(),
                comparison.slave_pic_exact(),
                comparison.ioapic_exact(),
                comparison.lapic_exact()
            ),
        ));
    }
    Ok(())
}

fn restore_full_controller_components_with<E, M, S, I, L>(
    mut restore_master: M,
    mut restore_slave: S,
    mut restore_ioapic: I,
    mut restore_lapic: L,
) -> Result<(), E>
where
    M: FnMut() -> Result<(), E>,
    S: FnMut() -> Result<(), E>,
    I: FnMut() -> Result<(), E>,
    L: FnMut() -> Result<(), E>,
{
    restore_master()?;
    restore_slave()?;
    restore_ioapic()?;
    restore_lapic()?;
    Ok(())
}

#[cfg(test)]
mod full_controller_checkpoint_tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn full_controller_restore_order_is_explicit_and_fail_closed() {
        let sequence = RefCell::new(Vec::new());
        restore_full_controller_components_with(
            || {
                sequence.borrow_mut().push("master");
                Ok::<_, &'static str>(())
            },
            || {
                sequence.borrow_mut().push("slave");
                Ok::<_, &'static str>(())
            },
            || {
                sequence.borrow_mut().push("ioapic");
                Ok::<_, &'static str>(())
            },
            || {
                sequence.borrow_mut().push("lapic");
                Ok::<_, &'static str>(())
            },
        )
        .unwrap();
        assert_eq!(&*sequence.borrow(), &["master", "slave", "ioapic", "lapic"]);

        sequence.borrow_mut().clear();
        let error = restore_full_controller_components_with(
            || {
                sequence.borrow_mut().push("master");
                Ok::<_, &'static str>(())
            },
            || {
                sequence.borrow_mut().push("slave");
                Ok::<_, &'static str>(())
            },
            || {
                sequence.borrow_mut().push("ioapic");
                Err::<(), _>("ioapic failure")
            },
            || {
                sequence.borrow_mut().push("lapic");
                Ok::<_, &'static str>(())
            },
        )
        .unwrap_err();
        assert_eq!(error, "ioapic failure");
        assert_eq!(&*sequence.borrow(), &["master", "slave", "ioapic"]);
    }

    #[test]
    fn deterministic_guest_preserves_debug_capture_and_two_controller_resume_contract() {
        assert_eq!(FULL_CONTROLLER_GUEST_BYTES.len(), 74);
        assert_eq!(
            &FULL_CONTROLLER_GUEST_BYTES[45..49],
            &[0xb0, CONTROLLER_CHECKPOINT_CAPTURE_ARM_BYTE, 0xe6, 0xe9]
        );
        assert_eq!(FULL_CONTROLLER_GUEST_BYTES[49], 0x90);
        assert_eq!(
            CONTROLLER_CHECKPOINT_CAPTURE_RIP,
            CONTROLLER_CHECKPOINT_ENTRY.get() + 49
        );
        assert_eq!(&FULL_CONTROLLER_GUEST_BYTES[50..52], &[0xfb, 0x90]);
        assert_eq!(&FULL_CONTROLLER_GUEST_BYTES[69..73], &[0xb0, b'D', 0xe6, 0xe9]);
        assert_eq!(FULL_CONTROLLER_GUEST_BYTES[73], 0xf4);
        assert_eq!(FULL_CONTROLLER_SLAVE_HANDLER_BYTES.len(), 14);
        assert_eq!(FULL_CONTROLLER_IOAPIC_HANDLER_BYTES.len(), 6);
        assert_eq!(FULL_CONTROLLER_CHECKPOINT_PROOF, b"ASBJMD");
    }
}
