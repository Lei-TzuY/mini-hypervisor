pub const TASK_WAIT_CHANNEL_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x300d0);
pub const TASK_WAIT_MISMATCH_COUNT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x300d1);
pub const TASK_WAIT_WAKE_COUNT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x300d2);
pub const TASK_WAIT_LAST_ATTEMPT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x300d3);
pub const TASK_WAIT_CHANNEL_NONE: u8 = 0;
pub const TASK_WAIT_CHANNEL_A: u8 = 0x11;
pub const TASK_WAIT_WRONG_CHANNEL: u8 = 0x22;
pub const TASK_WAIT_CHANNEL_PROOF: &[u8; 10] = b"K1AXPW0BRD";
pub const TASK_WAIT_DIRTY_CAPTURE_STAGES: &[u8; 4] = b"KXWD";
pub const TASK_WAIT_CHECKPOINT_CAPTURE_RIP: u64 = TASK_WAKE_ARM_HANDLER.get() + 0x3e;

const WAIT_TIMER_DELAY_MILLIS: u64 = 10;
const WAIT_WATCHDOG_SECONDS: u64 = 5;

const WAIT_BLOCK_HANDLER_BYTES: [u8; 25] = [
    0xc6, 0x04, 0x25, 0xc0, 0x00, 0x03, 0x00, 0x00, 0xc6, 0x04, 0x25, 0xd0, 0x00, 0x03, 0x00, 0x11,
    0xb0, b'K', 0xe6, 0xe9, 0xe9, 0xe7, 0x3f, 0x00, 0x00,
];

const WAIT_WAKE_ARM_HANDLER_BYTES: [u8; 105] = [
    0xc6, 0x04, 0x25, 0xd3, 0x00, 0x03, 0x00, 0x22, 0x80, 0x3c, 0x25, 0xd0, 0x00, 0x03, 0x00, 0x22,
    0x0f, 0x84, 0x4e, 0x00, 0x00, 0x00, 0xfe, 0x04, 0x25, 0xd1, 0x00, 0x03, 0x00, 0x80, 0x3c, 0x25,
    0xc0, 0x00, 0x03, 0x00, 0x00, 0x0f, 0x85, 0x39, 0x00, 0x00, 0x00, 0x80, 0x3c, 0x25, 0xd0, 0x00,
    0x03, 0x00, 0x11, 0x0f, 0x85, 0x2b, 0x00, 0x00, 0x00, 0xb0, b'X', 0xe6, 0xe9, 0xb0, b'P', 0xe6,
    0xe9, 0xfb, 0xf4, 0x80, 0x3c, 0x25, 0xc0, 0x00, 0x03, 0x00, 0x01, 0x0f, 0x85, 0x13, 0x00, 0x00,
    0x00, 0x80, 0x3c, 0x25, 0xd0, 0x00, 0x03, 0x00, 0x00, 0x0f, 0x85, 0x05, 0x00, 0x00, 0x00, 0xe9,
    0x9c, 0x1f, 0x00, 0x00, 0xb0, b'F', 0xe6, 0xe9, 0xf4,
];

const WAIT_WAKE_ARM_CHECKPOINT_HANDLER_BYTES: [u8; 106] = [
    0xc6, 0x04, 0x25, 0xd3, 0x00, 0x03, 0x00, 0x22, 0x80, 0x3c, 0x25, 0xd0, 0x00, 0x03, 0x00, 0x22,
    0x0f, 0x84, 0x4f, 0x00, 0x00, 0x00, 0xfe, 0x04, 0x25, 0xd1, 0x00, 0x03, 0x00, 0x80, 0x3c, 0x25,
    0xc0, 0x00, 0x03, 0x00, 0x00, 0x0f, 0x85, 0x3a, 0x00, 0x00, 0x00, 0x80, 0x3c, 0x25, 0xd0, 0x00,
    0x03, 0x00, 0x11, 0x0f, 0x85, 0x2c, 0x00, 0x00, 0x00, 0xb0, b'X', 0xe6, 0xe9, 0xf4, 0xb0, b'P',
    0xe6, 0xe9, 0xfb, 0xf4, 0x80, 0x3c, 0x25, 0xc0, 0x00, 0x03, 0x00, 0x01, 0x0f, 0x85, 0x13, 0x00,
    0x00, 0x00, 0x80, 0x3c, 0x25, 0xd0, 0x00, 0x03, 0x00, 0x00, 0x0f, 0x85, 0x05, 0x00, 0x00, 0x00,
    0xe9, 0x9b, 0x1f, 0x00, 0x00, 0xb0, b'F', 0xe6, 0xe9, 0xf4,
];

const WAIT_WAKE_TIMER_HANDLER_BYTES: [u8; 65] = [
    0xc6, 0x04, 0x25, 0xd3, 0x00, 0x03, 0x00, 0x11, 0x80, 0x3c, 0x25, 0xd0, 0x00, 0x03, 0x00, 0x11,
    0x0f, 0x85, 0x21, 0x00, 0x00, 0x00, 0xc6, 0x04, 0x25, 0xc0, 0x00, 0x03, 0x00, 0x01, 0xc6, 0x04,
    0x25, 0xd0, 0x00, 0x03, 0x00, 0x00, 0xfe, 0x04, 0x25, 0xd2, 0x00, 0x03, 0x00, 0xb0, b'W', 0xe6,
    0xe9, 0xb0, 0x20, 0xe6, 0x20, 0x48, 0xcf, 0xb0, b'F', 0xe6, 0xe9, 0xb0, 0x20, 0xe6, 0x20, 0x48,
    0xcf,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitChannelSnapshot {
    task_a_state: TaskRunState,
    owner: u8,
    mismatch_count: u8,
    wake_count: u8,
    last_attempt: u8,
}

impl WaitChannelSnapshot {
    #[must_use]
    pub const fn task_a_state(self) -> TaskRunState {
        self.task_a_state
    }

    #[must_use]
    pub const fn owner(self) -> u8 {
        self.owner
    }

    #[must_use]
    pub const fn mismatch_count(self) -> u8 {
        self.mismatch_count
    }

    #[must_use]
    pub const fn wake_count(self) -> u8 {
        self.wake_count
    }

    #[must_use]
    pub const fn last_attempt(self) -> u8 {
        self.last_attempt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WaitChannelModel {
    state: TaskRunState,
    owner: u8,
    mismatch_count: u8,
    wake_count: u8,
    last_attempt: u8,
}

impl WaitChannelModel {
    const fn new() -> Self {
        Self {
            state: TaskRunState::Runnable,
            owner: TASK_WAIT_CHANNEL_NONE,
            mismatch_count: 0,
            wake_count: 0,
            last_attempt: TASK_WAIT_CHANNEL_NONE,
        }
    }

    fn block(&mut self, channel: u8) -> Result<(), Error> {
        if channel == TASK_WAIT_CHANNEL_NONE {
            return Err(verification_error(
                "bounded wait-channel block",
                "channel zero is reserved for no owner",
            ));
        }
        if self.state != TaskRunState::Runnable || self.owner != TASK_WAIT_CHANNEL_NONE {
            return Err(verification_error(
                "bounded wait-channel block",
                "task must be runnable and unowned before blocking",
            ));
        }
        self.state = TaskRunState::Blocked;
        self.owner = channel;
        Ok(())
    }

    fn wake(&mut self, channel: u8) -> Result<bool, Error> {
        if channel == TASK_WAIT_CHANNEL_NONE {
            return Err(verification_error(
                "bounded wait-channel wake",
                "channel zero cannot wake a task",
            ));
        }
        if self.state != TaskRunState::Blocked || self.owner == TASK_WAIT_CHANNEL_NONE {
            return Err(verification_error(
                "bounded wait-channel wake",
                "wake requires a blocked task with an owning channel",
            ));
        }
        self.last_attempt = channel;
        if self.owner != channel {
            self.mismatch_count = self
                .mismatch_count
                .checked_add(1)
                .ok_or_else(|| verification_error("bounded wait-channel wake", "mismatch count overflow"))?;
            return Ok(false);
        }
        self.owner = TASK_WAIT_CHANNEL_NONE;
        self.state = TaskRunState::Runnable;
        self.wake_count = self
            .wake_count
            .checked_add(1)
            .ok_or_else(|| verification_error("bounded wait-channel wake", "wake count overflow"))?;
        Ok(true)
    }

    const fn snapshot(self) -> WaitChannelSnapshot {
        WaitChannelSnapshot {
            task_a_state: self.state,
            owner: self.owner,
            mismatch_count: self.mismatch_count,
            wake_count: self.wake_count,
            last_attempt: self.last_attempt,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitChannelGuestResult {
    gsi: u32,
    vector: u8,
    lapic_spiv: u32,
    lapic_lint0: u32,
    armed_rflags: u64,
    blocked_wait: WaitChannelSnapshot,
    mismatch_wait: WaitChannelSnapshot,
    wake_wait: WaitChannelSnapshot,
    final_wait: WaitChannelSnapshot,
    first_selection: RunnableQueueSnapshot,
    second_selection: RunnableQueueSnapshot,
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    task_a: TaskContextSnapshot,
    task_b: TaskContextSnapshot,
    terminal: TaskTerminalObservation,
    final_cr3: u64,
    final_r12: u64,
    task_a_stack_marker: u8,
    task_b_stack_marker: u8,
    first_context_pte: u64,
    second_context_pte: u64,
}

impl WaitChannelGuestResult {
    #[must_use]
    pub const fn gsi(&self) -> u32 { self.gsi }
    #[must_use]
    pub const fn vector(&self) -> u8 { self.vector }
    #[must_use]
    pub const fn lapic_spiv(&self) -> u32 { self.lapic_spiv }
    #[must_use]
    pub const fn lapic_lint0(&self) -> u32 { self.lapic_lint0 }
    #[must_use]
    pub const fn armed_rflags(&self) -> u64 { self.armed_rflags }
    #[must_use]
    pub const fn blocked_wait(&self) -> WaitChannelSnapshot { self.blocked_wait }
    #[must_use]
    pub const fn mismatch_wait(&self) -> WaitChannelSnapshot { self.mismatch_wait }
    #[must_use]
    pub const fn wake_wait(&self) -> WaitChannelSnapshot { self.wake_wait }
    #[must_use]
    pub const fn final_wait(&self) -> WaitChannelSnapshot { self.final_wait }
    #[must_use]
    pub const fn first_selection(&self) -> RunnableQueueSnapshot { self.first_selection }
    #[must_use]
    pub const fn second_selection(&self) -> RunnableQueueSnapshot { self.second_selection }
    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] { &self.io_exits }
    #[must_use]
    pub fn proof(&self) -> &[u8] { &self.proof }
    #[must_use]
    pub const fn task_a(&self) -> TaskContextSnapshot { self.task_a }
    #[must_use]
    pub const fn task_b(&self) -> TaskContextSnapshot { self.task_b }
    #[must_use]
    pub const fn terminal(&self) -> TaskTerminalObservation { self.terminal }
    #[must_use]
    pub const fn final_cr3(&self) -> u64 { self.final_cr3 }
    #[must_use]
    pub const fn final_r12(&self) -> u64 { self.final_r12 }
    #[must_use]
    pub const fn task_a_stack_marker(&self) -> u8 { self.task_a_stack_marker }
    #[must_use]
    pub const fn task_b_stack_marker(&self) -> u8 { self.task_b_stack_marker }
    #[must_use]
    pub const fn first_context_pte(&self) -> u64 { self.first_context_pte }
    #[must_use]
    pub const fn second_context_pte(&self) -> u64 { self.second_context_pte }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitChannelDirtyCapture {
    stage: u8,
    bitmap: Vec<u64>,
    wait: WaitChannelSnapshot,
}

impl WaitChannelDirtyCapture {
    #[must_use]
    pub const fn stage(&self) -> u8 { self.stage }
    #[must_use]
    pub fn bitmap(&self) -> &[u64] { &self.bitmap }
    #[must_use]
    pub const fn wait(&self) -> WaitChannelSnapshot { self.wait }
    #[must_use]
    pub fn context_page_dirty(&self) -> bool {
        dirty_bitmap_contains_gpa(&self.bitmap, TASK_CONTEXT_PAGE_ADDR)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitChannelDirtyGuestResult {
    guest: WaitChannelGuestResult,
    captures: Vec<WaitChannelDirtyCapture>,
}

impl WaitChannelDirtyGuestResult {
    #[must_use]
    pub const fn guest(&self) -> &WaitChannelGuestResult { &self.guest }
    #[must_use]
    pub fn captures(&self) -> &[WaitChannelDirtyCapture] { &self.captures }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitChannelCheckpointOwnership {
    wait: WaitChannelSnapshot,
    queue: RunnableQueueSnapshot,
    task_a: TaskContextSnapshot,
    task_b: TaskContextSnapshot,
}

impl WaitChannelCheckpointOwnership {
    #[must_use]
    pub const fn wait(self) -> WaitChannelSnapshot { self.wait }
    #[must_use]
    pub const fn queue(self) -> RunnableQueueSnapshot { self.queue }
    #[must_use]
    pub const fn task_a(self) -> TaskContextSnapshot { self.task_a }
    #[must_use]
    pub const fn task_b(self) -> TaskContextSnapshot { self.task_b }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitChannelCheckpointComparison {
    machine: crate::state_snapshot::BoundedCheckpointComparison,
    wait_exact: bool,
    queue_exact: bool,
    task_a_exact: bool,
    task_b_exact: bool,
}

impl WaitChannelCheckpointComparison {
    #[must_use]
    pub const fn machine(&self) -> &crate::state_snapshot::BoundedCheckpointComparison {
        &self.machine
    }
    #[must_use]
    pub const fn wait_exact(&self) -> bool { self.wait_exact }
    #[must_use]
    pub const fn queue_exact(&self) -> bool { self.queue_exact }
    #[must_use]
    pub const fn task_a_exact(&self) -> bool { self.task_a_exact }
    #[must_use]
    pub const fn task_b_exact(&self) -> bool { self.task_b_exact }
    #[must_use]
    pub fn ownership_exact(&self) -> bool {
        self.wait_exact && self.queue_exact && self.task_a_exact && self.task_b_exact
    }
    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.machine.is_exact_match() && self.ownership_exact()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitChannelCheckpointGuestResult {
    guest: WaitChannelGuestResult,
    checkpoint_report: VmExitReport,
    captured: WaitChannelCheckpointOwnership,
    corruption: WaitChannelCheckpointComparison,
    restored: WaitChannelCheckpointComparison,
}

impl WaitChannelCheckpointGuestResult {
    #[must_use]
    pub const fn guest(&self) -> &WaitChannelGuestResult { &self.guest }
    #[must_use]
    pub const fn checkpoint_report(&self) -> VmExitReport { self.checkpoint_report }
    #[must_use]
    pub const fn captured(&self) -> WaitChannelCheckpointOwnership { self.captured }
    #[must_use]
    pub const fn corruption(&self) -> &WaitChannelCheckpointComparison { &self.corruption }
    #[must_use]
    pub const fn restored(&self) -> &WaitChannelCheckpointComparison { &self.restored }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitChannelRunMode {
    Plain,
    Dirty,
    Checkpoint,
}

impl WaitChannelRunMode {
    const fn track_dirty(self) -> bool { matches!(self, Self::Dirty) }
    const fn checkpoint(self) -> bool { matches!(self, Self::Checkpoint) }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WaitChannelCheckpointEvidence {
    checkpoint_report: VmExitReport,
    captured: WaitChannelCheckpointOwnership,
    corruption: WaitChannelCheckpointComparison,
    restored: WaitChannelCheckpointComparison,
}

struct WaitChannelRunArtifacts {
    guest: WaitChannelGuestResult,
    dirty_captures: Vec<WaitChannelDirtyCapture>,
    checkpoint: Option<WaitChannelCheckpointEvidence>,
}

pub fn run_bounded_wait_channel_guest(config: VmConfig) -> Result<WaitChannelGuestResult, Error> {
    let artifacts = run_bounded_wait_channel_guest_inner(config, WaitChannelRunMode::Plain)?;
    debug_assert!(artifacts.dirty_captures.is_empty());
    debug_assert!(artifacts.checkpoint.is_none());
    Ok(artifacts.guest)
}

pub fn run_bounded_wait_channel_dirty_guest(
    config: VmConfig,
) -> Result<WaitChannelDirtyGuestResult, Error> {
    let artifacts = run_bounded_wait_channel_guest_inner(config, WaitChannelRunMode::Dirty)?;
    if artifacts.dirty_captures.len() != TASK_WAIT_DIRTY_CAPTURE_STAGES.len()
        || artifacts
            .dirty_captures
            .iter()
            .map(WaitChannelDirtyCapture::stage)
            .ne(TASK_WAIT_DIRTY_CAPTURE_STAGES.iter().copied())
    {
        return Err(verification_error(
            "wait-channel dirty capture sequence",
            format!(
                "expected stages {:?}, got {:?}",
                TASK_WAIT_DIRTY_CAPTURE_STAGES,
                artifacts
                    .dirty_captures
                    .iter()
                    .map(WaitChannelDirtyCapture::stage)
                    .collect::<Vec<_>>()
            ),
        ));
    }
    Ok(WaitChannelDirtyGuestResult {
        guest: artifacts.guest,
        captures: artifacts.dirty_captures,
    })
}

pub fn run_bounded_wait_channel_checkpoint_guest(
    config: VmConfig,
) -> Result<WaitChannelCheckpointGuestResult, Error> {
    let artifacts = run_bounded_wait_channel_guest_inner(config, WaitChannelRunMode::Checkpoint)?;
    if !artifacts.dirty_captures.is_empty() {
        return Err(verification_error(
            "wait-channel checkpoint mode",
            "checkpoint mode unexpectedly produced dirty-log captures",
        ));
    }
    let Some(checkpoint) = artifacts.checkpoint else {
        return Err(verification_error(
            "wait-channel checkpoint mode",
            "checkpoint mode completed without checkpoint evidence",
        ));
    };
    Ok(WaitChannelCheckpointGuestResult {
        guest: artifacts.guest,
        checkpoint_report: checkpoint.checkpoint_report,
        captured: checkpoint.captured,
        corruption: checkpoint.corruption,
        restored: checkpoint.restored,
    })
}

fn run_bounded_wait_channel_guest_inner(
    config: VmConfig,
    mode: WaitChannelRunMode,
) -> Result<WaitChannelRunArtifacts, Error> {
    let kernel_bytes = queue_kernel_bytes();
    let kernel = FlatGuestImage::new(PRIVILEGE_KERNEL_ENTRY, PRIVILEGE_KERNEL_ENTRY, &kernel_bytes)?;
    let task_a = FlatGuestImage::new(PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_ENTRY, &QUEUE_TASK_A_BYTES)?;
    let task_b = FlatGuestImage::new(
        ADDRESS_SPACE_B_USER_CODE_BACKING,
        ADDRESS_SPACE_B_USER_CODE_BACKING,
        &QUEUE_TASK_B_BYTES,
    )?;
    let scheduler = FlatGuestImage::new(
        PRIVILEGE_RETURN_HANDLER,
        PRIVILEGE_RETURN_HANDLER,
        &SCHEDULER_HANDLER_BYTES,
    )?;
    let terminal = FlatGuestImage::new(
        PRIVILEGE_TERMINAL_HANDLER,
        PRIVILEGE_TERMINAL_HANDLER,
        &TERMINAL_HANDLER_BYTES,
    )?;
    let block_handler = FlatGuestImage::new(
        TASK_BLOCK_HANDLER,
        TASK_BLOCK_HANDLER,
        &WAIT_BLOCK_HANDLER_BYTES,
    )?;
    let arm_handler_bytes: &[u8] = if mode.checkpoint() {
        &WAIT_WAKE_ARM_CHECKPOINT_HANDLER_BYTES
    } else {
        &WAIT_WAKE_ARM_HANDLER_BYTES
    };
    let arm_handler = FlatGuestImage::new(
        TASK_WAKE_ARM_HANDLER,
        TASK_WAKE_ARM_HANDLER,
        arm_handler_bytes,
    )?;
    let wake_handler = FlatGuestImage::new(
        TASK_WAKE_TIMER_HANDLER,
        TASK_WAKE_TIMER_HANDLER,
        &WAIT_WAKE_TIMER_HANDLER_BYTES,
    )?;
    let queue_handler = FlatGuestImage::new(TASK_QUEUE_HANDLER, TASK_QUEUE_HANDLER, &QUEUE_HANDLER_BYTES)?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = AddressSpaceSwitchLayout::new(memory.region())
        .expect("fixed bounded wait-channel layout remains valid");
    layout.install_tables(&mut memory)?;
    install_queue_gates(&mut memory, &layout)?;
    kernel.load(&mut memory)?;
    task_a.load(&mut memory)?;
    task_b.load(&mut memory)?;
    scheduler.load(&mut memory)?;
    terminal.load(&mut memory)?;
    block_handler.load(&mut memory)?;
    arm_handler.load(&mut memory)?;
    wake_handler.load(&mut memory)?;
    queue_handler.load(&mut memory)?;
    memory.write(TASK_CONTEXT_PAGE_ADDR, &[0; LONG_MODE_PAGE_SIZE as usize])?;
    write_context(
        &mut memory,
        TASK_B_CONTEXT_ADDR,
        TaskContextSnapshot {
            cr3: ADDRESS_SPACE_B_PML4_ADDR.get(),
            rip: PRIVILEGE_USER_ENTRY.get(),
            rsp: TASK_B_INITIAL_RSP,
            rflags: TASK_USER_RFLAGS,
            r12: TASK_B_INITIAL_R12,
            save_count: 0,
        },
    )?;
    initialize_queue_metadata(&mut memory)?;
    initialize_wait_channel_metadata(&mut memory)?;
    let dirty_slot = if mode.track_dirty() {
        Some(crate::kvm::register_guest_memory_with_dirty_log(&mut vm, memory)?)
    } else {
        vm.register_guest_memory(memory)?;
        None
    };

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(layout.privilege_layout())?;
    let lapic = vcpu.configure_legacy_pic_extint()?;
    let mut port_io = PortIoBus::with_debug_port();
    let mut dirty_captures = Vec::new();
    let mut checkpoint_evidence = None;
    if let Some(slot) = dirty_slot {
        drain_wait_dirty_baseline(&vm, slot)?;
    }

    let blocked_io = run_queue_debug_output(&mut vcpu, &mut port_io, b'K', "wait-channel block A")?;
    let blocked_wait = read_wait_channel_snapshot(
        vm.guest_memory().expect("registered wait-channel memory remains VM-owned"),
    )?;
    require_wait_blocked(blocked_wait)?;
    if let Some(slot) = dirty_slot {
        dirty_captures.push(capture_wait_dirty(&vm, slot, b'K', blocked_wait)?);
    }

    let first_select_io = run_queue_debug_output(
        &mut vcpu,
        &mut port_io,
        b'1',
        "wait-channel queue selects B",
    )?;
    let first_selection = read_queue_snapshot(
        vm.guest_memory().expect("registered wait-channel memory remains VM-owned"),
    )?;
    require_first_queue_selection(first_selection)?;

    let scheduler_a_io = run_queue_debug_output(
        &mut vcpu,
        &mut port_io,
        b'A',
        "wait-channel B scheduler handoff",
    )?;
    let mismatch_io = run_queue_debug_output(
        &mut vcpu,
        &mut port_io,
        b'X',
        "wait-channel wrong wake attempt",
    )?;
    let mismatch_wait = read_wait_channel_snapshot(
        vm.guest_memory().expect("registered wait-channel memory remains VM-owned"),
    )?;
    require_wait_mismatch(mismatch_wait)?;
    if let Some(slot) = dirty_slot {
        dirty_captures.push(capture_wait_dirty(&vm, slot, b'X', mismatch_wait)?);
    }

    if mode.checkpoint() {
        checkpoint_evidence = Some(capture_mutate_restore_wait_checkpoint(
            &backend,
            &vm,
            &mut vcpu,
            &layout,
            mismatch_wait,
            first_selection,
        )?);
    }

    let armed_io = run_queue_debug_output(&mut vcpu, &mut port_io, b'P', "wait-channel wake arm")?;
    let armed = vcpu.registers()?;
    require_queue_interrupt_disabled_flags("wait-channel wake arm state", armed.rflags)?;

    let timer_irq = vm
        .duplicate_irq_line_handle()
        .map_err(|source| queue_vm_error("duplicate wait-channel wake IRQ handle", source))?;
    let watchdog_irq = vm
        .duplicate_irq_line_handle()
        .map_err(|source| queue_vm_error("duplicate wait-channel watchdog handle", source))?;
    timer_irq
        .set_gsi_level(TASK_WAKE_GSI, false)
        .map_err(|source| queue_vm_error("preflight wait-channel IRQ line", source))?;
    watchdog_irq
        .set_gsi_level(TASK_WAKE_GSI, false)
        .map_err(|source| queue_vm_error("preflight wait-channel watchdog line", source))?;

    let timer_worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(WAIT_TIMER_DELAY_MILLIS));
        timer_irq.pulse_gsi_edge(TASK_WAKE_GSI)
    });
    let (watchdog_cancel_tx, watchdog_cancel_rx) = std::sync::mpsc::channel::<()>();
    let watchdog_worker = std::thread::spawn(move || -> io::Result<bool> {
        match watchdog_cancel_rx.recv_timeout(std::time::Duration::from_secs(WAIT_WATCHDOG_SECONDS)) {
            Ok(()) => Ok(false),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                watchdog_irq.pulse_gsi_edge(TASK_WAKE_GSI)?;
                Ok(true)
            }
        }
    });

    let execution = (|| -> Result<_, Error> {
        let wake_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'W',
            "wait-channel correct external wake",
        )?;
        let wake_wait = read_wait_channel_snapshot(
            vm.guest_memory().expect("registered wait-channel memory remains VM-owned"),
        )?;
        require_wait_woken(wake_wait)?;
        if let Some(slot) = dirty_slot {
            dirty_captures.push(capture_wait_dirty(&vm, slot, b'W', wake_wait)?);
        }
        let second_select_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'0',
            "wait-channel queue selects woken A",
        )?;
        let second_selection = read_queue_snapshot(
            vm.guest_memory().expect("registered wait-channel memory remains VM-owned"),
        )?;
        require_second_queue_selection(second_selection)?;
        let scheduler_b_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'B',
            "wait-channel A scheduler handoff",
        )?;
        let restored_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'R',
            "wait-channel restored A",
        )?;
        let done_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'D',
            "wait-channel completion",
        )?;
        let done_wait = read_wait_channel_snapshot(
            vm.guest_memory().expect("registered wait-channel memory remains VM-owned"),
        )?;
        require_wait_woken(done_wait)?;
        if let Some(slot) = dirty_slot {
            dirty_captures.push(capture_wait_dirty(&vm, slot, b'D', done_wait)?);
        }
        Ok((wake_io, wake_wait, second_select_io, second_selection, scheduler_b_io, restored_io, done_io))
    })();

    let _ = watchdog_cancel_tx.send(());
    let timer_result = timer_worker.join().map_err(|_| {
        verification_error("join wait-channel timer", "wait-channel timer worker panicked")
    })?;
    let watchdog_fired = watchdog_worker
        .join()
        .map_err(|_| verification_error("join wait-channel watchdog", "wait-channel watchdog panicked"))?
        .map_err(|source| queue_vm_error("wait-channel watchdog GSI", source))?;
    timer_result.map_err(|source| queue_vm_error("wait-channel wake GSI", source))?;
    if watchdog_fired {
        return Err(verification_error(
            "wait-channel watchdog",
            "watchdog injected fallback GSI; correct-channel wake was not independently proven",
        ));
    }

    let (wake_io, wake_wait, second_select_io, second_selection, scheduler_b_io, restored_io, done_io) =
        execution?;
    let io_exits = vec![
        blocked_io,
        first_select_io,
        scheduler_a_io,
        mismatch_io,
        armed_io,
        wake_io,
        second_select_io,
        scheduler_b_io,
        restored_io,
        done_io,
    ];
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != TASK_WAIT_CHANNEL_PROOF || io_exits.len() != TASK_WAIT_CHANNEL_PROOF.len() {
        return Err(verification_error(
            "bounded wait-channel proof",
            format!(
                "expected {:?} across {} exits, got {:?} across {} exits",
                TASK_WAIT_CHANNEL_PROOF,
                TASK_WAIT_CHANNEL_PROOF.len(),
                proof,
                io_exits.len()
            ),
        ));
    }

    let final_regs = vcpu.capture_register_snapshot()?;
    let final_special = vcpu.capture_special_register_snapshot()?;
    let guest_memory = vm.guest_memory().expect("registered wait-channel memory remains VM-owned");
    let final_wait = read_wait_channel_snapshot(guest_memory)?;
    require_wait_woken(final_wait)?;
    let task_a_context = read_context(guest_memory, TASK_A_CONTEXT_ADDR)?;
    let task_b_context = read_context(guest_memory, TASK_B_CONTEXT_ADDR)?;
    let terminal_observation = read_terminal_observation(guest_memory)?;
    let task_a_stack_marker = read_byte(guest_memory, TASK_A_STACK_MARKER_PHYS)?;
    let task_b_stack_marker = read_byte(guest_memory, TASK_B_STACK_MARKER_PHYS)?;
    let first_context_pte = read_pte(guest_memory, PRIVILEGE_PT_ADDR, TASK_CONTEXT_PAGE_ADDR.get())?;
    let second_context_pte = read_pte(
        guest_memory,
        ADDRESS_SPACE_B_PT_ADDR,
        TASK_CONTEXT_PAGE_ADDR.get(),
    )?;
    validate_queue_context_state(
        task_a_context,
        task_b_context,
        terminal_observation,
        final_special.cr3(),
        final_regs.r12(),
        task_a_stack_marker,
        task_b_stack_marker,
        first_context_pte,
        second_context_pte,
    )?;

    Ok(WaitChannelRunArtifacts {
        guest: WaitChannelGuestResult {
            gsi: TASK_WAKE_GSI,
            vector: TASK_WAKE_TIMER_VECTOR,
            lapic_spiv: lapic.spiv(),
            lapic_lint0: lapic.lint0(),
            armed_rflags: armed.rflags,
            blocked_wait,
            mismatch_wait,
            wake_wait,
            final_wait,
            first_selection,
            second_selection,
            io_exits,
            proof,
            task_a: task_a_context,
            task_b: task_b_context,
            terminal: terminal_observation,
            final_cr3: final_special.cr3(),
            final_r12: final_regs.r12(),
            task_a_stack_marker,
            task_b_stack_marker,
            first_context_pte,
            second_context_pte,
        },
        dirty_captures,
        checkpoint: checkpoint_evidence,
    })
}

fn capture_mutate_restore_wait_checkpoint(
    backend: &KvmBackend,
    vm: &crate::kvm::Vm,
    vcpu: &mut crate::vcpu::Vcpu,
    layout: &AddressSpaceSwitchLayout,
    mismatch_wait: WaitChannelSnapshot,
    first_selection: RunnableQueueSnapshot,
) -> Result<WaitChannelCheckpointEvidence, Error> {
    let mut no_io = PortIoBus::empty();
    let checkpoint_execution = run_vcpu_until_stopped(vcpu, &mut no_io, 1)?;
    let checkpoint_report = checkpoint_execution.report();
    require_wait_checkpoint_hlt(checkpoint_report)?;
    if !checkpoint_execution.io_exits().is_empty() {
        return Err(verification_error(
            "wait-channel checkpoint boundary",
            "checkpoint HLT unexpectedly serviced port I/O",
        ));
    }

    let msr_policy = crate::kvm::msr::GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty wait checkpoint MSR policy is valid by construction");
    let machine = crate::state_snapshot::BoundedVcpuPageCheckpoint::capture(
        vcpu,
        &msr_policy,
        vm.guest_memory()
            .expect("registered wait-channel memory remains VM-owned"),
        TASK_CONTEXT_PAGE_ADDR,
    )?;
    let captured = capture_wait_checkpoint_ownership(
        vm.guest_memory()
            .expect("registered wait-channel memory remains VM-owned"),
    )?;
    if captured.wait != mismatch_wait || captured.queue != first_selection {
        return Err(verification_error(
            "wait-channel checkpoint capture ownership",
            format!(
                "checkpoint ownership drifted before capture: wait {:?} vs {:?}, queue {:?} vs {:?}",
                captured.wait, mismatch_wait, captured.queue, first_selection
            ),
        ));
    }

    mutate_wait_checkpoint_ownership(
        vm.guest_memory_mut()
            .expect("registered wait-channel memory remains VM-owned"),
    )?;
    vcpu.initialize_long_mode_privilege(layout.privilege_layout())?;

    let corruption_machine = machine.verify(
        vcpu,
        vm.guest_memory()
            .expect("registered wait-channel memory remains VM-owned"),
    )?;
    let corruption_ownership = capture_wait_checkpoint_ownership(
        vm.guest_memory()
            .expect("registered wait-channel memory remains VM-owned"),
    )?;
    let corruption = compare_wait_checkpoint(&captured, corruption_machine, corruption_ownership);
    if corruption.machine().page_exact()
        || corruption.machine().vcpu().is_exact_match()
        || corruption.ownership_exact()
    {
        return Err(verification_error(
            "wait-channel checkpoint corruption proof",
            format!(
                "expected page, VCPU and typed ownership mismatch; got page_exact={} vcpu_exact={} ownership_exact={}",
                corruption.machine().page_exact(),
                corruption.machine().vcpu().is_exact_match(),
                corruption.ownership_exact()
            ),
        ));
    }

    let restored_machine = machine.restore_and_verify(
        vcpu,
        vm.guest_memory_mut()
            .expect("registered wait-channel memory remains VM-owned"),
    )?;
    let restored_ownership = capture_wait_checkpoint_ownership(
        vm.guest_memory()
            .expect("registered wait-channel memory remains VM-owned"),
    )?;
    let restored = compare_wait_checkpoint(&captured, restored_machine, restored_ownership);
    if !restored.is_exact_match() {
        return Err(verification_error(
            "wait-channel checkpoint restore verification",
            format!(
                "restore mismatch: page_exact={} vcpu_exact={} wait_exact={} queue_exact={} task_a_exact={} task_b_exact={}",
                restored.machine().page_exact(),
                restored.machine().vcpu().is_exact_match(),
                restored.wait_exact(),
                restored.queue_exact(),
                restored.task_a_exact(),
                restored.task_b_exact()
            ),
        ));
    }

    Ok(WaitChannelCheckpointEvidence {
        checkpoint_report,
        captured,
        corruption,
        restored,
    })
}

fn require_wait_checkpoint_hlt(report: VmExitReport) -> Result<(), Error> {
    if report.exit() != VcpuExit::Hlt
        || report.rip() != TASK_WAIT_CHECKPOINT_CAPTURE_RIP
        || report.rflags() & 0x2 != 0x2
    {
        return Err(verification_error(
            "wait-channel checkpoint boundary",
            format!(
                "expected HLT at rip={TASK_WAIT_CHECKPOINT_CAPTURE_RIP:#x} with architectural RFLAGS bit1, got {report}"
            ),
        ));
    }
    Ok(())
}

fn capture_wait_checkpoint_ownership(
    memory: &GuestMemory,
) -> Result<WaitChannelCheckpointOwnership, Error> {
    Ok(WaitChannelCheckpointOwnership {
        wait: read_wait_channel_snapshot(memory)?,
        queue: read_queue_snapshot(memory)?,
        task_a: read_context(memory, TASK_A_CONTEXT_ADDR)?,
        task_b: read_context(memory, TASK_B_CONTEXT_ADDR)?,
    })
}

fn compare_wait_checkpoint(
    expected: &WaitChannelCheckpointOwnership,
    machine: crate::state_snapshot::BoundedCheckpointComparison,
    observed: WaitChannelCheckpointOwnership,
) -> WaitChannelCheckpointComparison {
    WaitChannelCheckpointComparison {
        machine,
        wait_exact: observed.wait == expected.wait,
        queue_exact: observed.queue == expected.queue,
        task_a_exact: observed.task_a == expected.task_a,
        task_b_exact: observed.task_b == expected.task_b,
    }
}

fn mutate_wait_checkpoint_ownership(memory: &mut GuestMemory) -> Result<(), Error> {
    memory.write(TASK_QUEUE_A_STATE_ADDR, &[TaskRunState::Runnable as u8])?;
    memory.write(TASK_QUEUE_SELECTED_ADDR, &[RunnableTaskId::A as u8])?;
    memory.write(TASK_WAIT_CHANNEL_ADDR, &[TASK_WAIT_CHANNEL_NONE])?;
    memory.write(TASK_WAIT_MISMATCH_COUNT_ADDR, &[0])?;
    memory.write(TASK_WAIT_LAST_ATTEMPT_ADDR, &[TASK_WAIT_CHANNEL_NONE])?;
    Ok(())
}

fn drain_wait_dirty_baseline(
    vm: &crate::kvm::Vm,
    slot: crate::kvm::DirtyLogSlot0,
) -> Result<(), Error> {
    let setup_dirty = crate::kvm::harvest_dirty_log(vm, slot)?;
    let clean = crate::kvm::harvest_dirty_log(vm, slot)?;
    if clean.iter().any(|word| *word != 0) {
        return Err(verification_error(
            "wait-channel dirty baseline",
            format!("expected clean bitmap after draining setup dirtiness {setup_dirty:?}, got {clean:?}"),
        ));
    }
    Ok(())
}

fn capture_wait_dirty(
    vm: &crate::kvm::Vm,
    slot: crate::kvm::DirtyLogSlot0,
    stage: u8,
    wait: WaitChannelSnapshot,
) -> Result<WaitChannelDirtyCapture, Error> {
    let bitmap = crate::kvm::harvest_dirty_log(vm, slot)?;
    if !dirty_bitmap_contains_gpa(&bitmap, TASK_CONTEXT_PAGE_ADDR) {
        return Err(verification_error(
            "wait-channel dirty capture",
            format!(
                "stage {} did not mark supervisor task-context page {:#x} dirty: {bitmap:?}",
                char::from(stage),
                TASK_CONTEXT_PAGE_ADDR.get()
            ),
        ));
    }
    let clean = crate::kvm::harvest_dirty_log(vm, slot)?;
    if clean.iter().any(|word| *word != 0) {
        return Err(verification_error(
            "wait-channel dirty clear",
            format!("stage {} left uncleared dirty bits after harvest: {clean:?}", char::from(stage)),
        ));
    }
    Ok(WaitChannelDirtyCapture { stage, bitmap, wait })
}

fn dirty_bitmap_contains_gpa(bitmap: &[u64], address: GuestPhysAddr) -> bool {
    let page = address.get() / crate::memory::KVM_MEMORY_ALIGNMENT;
    let word_index = usize::try_from(page / 64).expect("guest page index fits host usize");
    let bit = page % 64;
    bitmap
        .get(word_index)
        .is_some_and(|word| word & (1_u64 << bit) != 0)
}

fn initialize_wait_channel_metadata(memory: &mut GuestMemory) -> Result<(), Error> {
    memory.write(TASK_WAIT_CHANNEL_ADDR, &[0, 0, 0, 0])?;
    Ok(())
}

fn read_wait_channel_snapshot(memory: &GuestMemory) -> Result<WaitChannelSnapshot, Error> {
    Ok(WaitChannelSnapshot {
        task_a_state: read_queue_state(memory, TASK_QUEUE_A_STATE_ADDR)?,
        owner: read_byte(memory, TASK_WAIT_CHANNEL_ADDR)?,
        mismatch_count: read_byte(memory, TASK_WAIT_MISMATCH_COUNT_ADDR)?,
        wake_count: read_byte(memory, TASK_WAIT_WAKE_COUNT_ADDR)?,
        last_attempt: read_byte(memory, TASK_WAIT_LAST_ATTEMPT_ADDR)?,
    })
}

fn require_wait_blocked(actual: WaitChannelSnapshot) -> Result<(), Error> {
    let mut model = WaitChannelModel::new();
    model.block(TASK_WAIT_CHANNEL_A)?;
    require_wait_snapshot("wait-channel blocked ownership", actual, model.snapshot())
}

fn require_wait_mismatch(actual: WaitChannelSnapshot) -> Result<(), Error> {
    let mut model = WaitChannelModel::new();
    model.block(TASK_WAIT_CHANNEL_A)?;
    if model.wake(TASK_WAIT_WRONG_CHANNEL)? {
        return Err(verification_error(
            "wait-channel model mismatch",
            "wrong channel unexpectedly woke the task",
        ));
    }
    require_wait_snapshot("wait-channel mismatch ownership", actual, model.snapshot())
}

fn require_wait_woken(actual: WaitChannelSnapshot) -> Result<(), Error> {
    let mut model = WaitChannelModel::new();
    model.block(TASK_WAIT_CHANNEL_A)?;
    let _ = model.wake(TASK_WAIT_WRONG_CHANNEL)?;
    if !model.wake(TASK_WAIT_CHANNEL_A)? {
        return Err(verification_error(
            "wait-channel model wake",
            "owning channel failed to wake the task",
        ));
    }
    require_wait_snapshot("wait-channel successful wake ownership", actual, model.snapshot())
}

fn require_wait_snapshot(
    stage: &'static str,
    actual: WaitChannelSnapshot,
    expected: WaitChannelSnapshot,
) -> Result<(), Error> {
    if actual != expected {
        return Err(verification_error(
            stage,
            format!("expected wait snapshot {expected:?}, got {actual:?}"),
        ));
    }
    Ok(())
}

const _: () = {
    assert!(TASK_WAIT_CHANNEL_ADDR.get() >= TASK_CONTEXT_PAGE_ADDR.get());
    assert!(TASK_WAIT_LAST_ATTEMPT_ADDR.get() < TASK_CONTEXT_PAGE_ADDR.get() + LONG_MODE_PAGE_SIZE);
};

#[cfg(test)]
mod wait_channel_tests {
    use super::*;

    #[test]
    fn wait_model_rejects_wrong_channel_without_releasing_owner() {
        let mut model = WaitChannelModel::new();
        model.block(TASK_WAIT_CHANNEL_A).unwrap();
        assert!(!model.wake(TASK_WAIT_WRONG_CHANNEL).unwrap());
        assert_eq!(
            model.snapshot(),
            WaitChannelSnapshot {
                task_a_state: TaskRunState::Blocked,
                owner: TASK_WAIT_CHANNEL_A,
                mismatch_count: 1,
                wake_count: 0,
                last_attempt: TASK_WAIT_WRONG_CHANNEL,
            }
        );
        assert!(model.wake(TASK_WAIT_CHANNEL_A).unwrap());
        assert_eq!(model.snapshot().task_a_state(), TaskRunState::Runnable);
        assert_eq!(model.snapshot().owner(), TASK_WAIT_CHANNEL_NONE);
        assert_eq!(model.snapshot().wake_count(), 1);
    }

    #[test]
    fn wait_model_rejects_zero_channel_double_block_and_wake_without_owner() {
        let mut model = WaitChannelModel::new();
        assert!(model.block(TASK_WAIT_CHANNEL_NONE).is_err());
        assert!(model.wake(TASK_WAIT_CHANNEL_A).is_err());
        model.block(TASK_WAIT_CHANNEL_A).unwrap();
        assert!(model.block(TASK_WAIT_CHANNEL_A).is_err());
    }

    #[test]
    fn wait_handlers_preserve_channel_and_existing_scheduler_contract() {
        assert_eq!(WAIT_BLOCK_HANDLER_BYTES.len(), 25);
        assert_eq!(WAIT_WAKE_ARM_HANDLER_BYTES.len(), 105);
        assert_eq!(WAIT_WAKE_TIMER_HANDLER_BYTES.len(), 65);
        assert_eq!(&WAIT_BLOCK_HANDLER_BYTES[8..16], &[0xc6, 0x04, 0x25, 0xd0, 0x00, 0x03, 0x00, 0x11]);
        assert!(WAIT_WAKE_ARM_HANDLER_BYTES
            .windows(4)
            .any(|window| window == [0xb0, b'X', 0xe6, 0xe9]));
        assert!(WAIT_WAKE_ARM_HANDLER_BYTES
            .windows(4)
            .any(|window| window == [0xb0, b'P', 0xe6, 0xe9]));
        assert_eq!(&WAIT_WAKE_ARM_HANDLER_BYTES[65..67], &[0xfb, 0xf4]);
        assert!(WAIT_WAKE_TIMER_HANDLER_BYTES
            .windows(4)
            .any(|window| window == [0xb0, b'W', 0xe6, 0xe9]));
        assert_eq!(QUEUE_TASK_A_BYTES.len(), 19);
        assert_eq!(QUEUE_TASK_B_BYTES.len(), 21);
        assert_eq!(QUEUE_HANDLER_BYTES.len(), 233);
    }

    #[test]
    fn checkpoint_arm_handler_commits_x_then_stops_before_p() {
        assert_eq!(WAIT_WAKE_ARM_CHECKPOINT_HANDLER_BYTES.len(), 106);
        assert_eq!(
            &WAIT_WAKE_ARM_CHECKPOINT_HANDLER_BYTES[0x39..0x3d],
            &[0xb0, b'X', 0xe6, 0xe9]
        );
        assert_eq!(WAIT_WAKE_ARM_CHECKPOINT_HANDLER_BYTES[0x3d], 0xf4);
        assert_eq!(
            &WAIT_WAKE_ARM_CHECKPOINT_HANDLER_BYTES[0x3e..0x42],
            &[0xb0, b'P', 0xe6, 0xe9]
        );
        assert_eq!(
            TASK_WAIT_CHECKPOINT_CAPTURE_RIP,
            TASK_WAKE_ARM_HANDLER.get() + 0x3e
        );
    }

    #[test]
    fn dirty_bitmap_tracks_the_supervisor_context_page() {
        let page = TASK_CONTEXT_PAGE_ADDR.get() / crate::memory::KVM_MEMORY_ALIGNMENT;
        assert_eq!(page, 48);
        let mut bitmap = vec![0_u64; 8];
        bitmap[0] = 1_u64 << page;
        assert!(dirty_bitmap_contains_gpa(&bitmap, TASK_CONTEXT_PAGE_ADDR));
        assert!(!dirty_bitmap_contains_gpa(&[0; 8], TASK_CONTEXT_PAGE_ADDR));
        assert_eq!(TASK_WAIT_DIRTY_CAPTURE_STAGES, b"KXWD");
    }
}
