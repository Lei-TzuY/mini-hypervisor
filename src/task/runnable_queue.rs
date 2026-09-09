use super::*;
use crate::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use crate::portio::PortIoService;
use crate::privilege::PRIVILEGE_IDT_ADDR;
use crate::vcpu::Vcpu;

pub const TASK_QUEUE_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x18000);
pub const TASK_QUEUE_A_STATE_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x300c0);
pub const TASK_QUEUE_B_STATE_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x300c1);
pub const TASK_QUEUE_ENTRY0_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x300c8);
pub const TASK_QUEUE_ENTRY1_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x300c9);
pub const TASK_QUEUE_HEAD_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x300ca);
pub const TASK_QUEUE_SELECTED_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x300cb);
pub const TASK_QUEUE_SKIP_COUNT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x300cc);
pub const TASK_RUNNABLE_QUEUE_PROOF: &[u8; 9] = b"K1APW0BRD";

const TASK_QUEUE_TIMER_DELAY_MILLIS: u64 = 10;
const TASK_QUEUE_WATCHDOG_SECONDS: u64 = 5;
const TASK_QUEUE_FAILURE_BYTE: u8 = b'F';
const X86_INTERRUPT_GATE_SIZE: u64 = 16;
const X86_KERNEL_CODE_SELECTOR: u16 = 0x08;
const X86_RING0_INTERRUPT_GATE: u8 = 0x8e;
const X86_RING3_INTERRUPT_GATE: u8 = 0xee;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnableTaskId {
    A = 0,
    B = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunnableQueueSnapshot {
    entry0: RunnableTaskId,
    entry1: RunnableTaskId,
    head: u8,
    selected: RunnableTaskId,
    skip_count: u8,
    task_a_state: TaskRunState,
    task_b_state: TaskRunState,
}

impl RunnableQueueSnapshot {
    #[must_use]
    pub const fn entry0(self) -> RunnableTaskId {
        self.entry0
    }

    #[must_use]
    pub const fn entry1(self) -> RunnableTaskId {
        self.entry1
    }

    #[must_use]
    pub const fn head(self) -> u8 {
        self.head
    }

    #[must_use]
    pub const fn selected(self) -> RunnableTaskId {
        self.selected
    }

    #[must_use]
    pub const fn skip_count(self) -> u8 {
        self.skip_count
    }

    #[must_use]
    pub const fn task_a_state(self) -> TaskRunState {
        self.task_a_state
    }

    #[must_use]
    pub const fn task_b_state(self) -> TaskRunState {
        self.task_b_state
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueModel {
    entries: [RunnableTaskId; 2],
    head: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueSelection {
    task: RunnableTaskId,
    skipped: u8,
}

impl QueueModel {
    fn new(entries: [RunnableTaskId; 2]) -> Result<Self, Error> {
        if entries[0] == entries[1] {
            return Err(verification_error(
                "bounded runnable queue configuration",
                "duplicate task IDs are not allowed in the two-entry queue",
            ));
        }
        Ok(Self { entries, head: 0 })
    }

    fn select_next(
        &mut self,
        task_a_state: TaskRunState,
        task_b_state: TaskRunState,
    ) -> Result<QueueSelection, Error> {
        let states = [task_a_state, task_b_state];
        let mut skipped = 0_u8;
        for _ in 0..self.entries.len() {
            let index = self.head;
            let task = self.entries[index];
            self.head = (self.head + 1) % self.entries.len();
            if states[task as usize] == TaskRunState::Runnable {
                return Ok(QueueSelection { task, skipped });
            }
            skipped = skipped
                .checked_add(1)
                .expect("two-entry skip count fits u8");
        }
        Err(verification_error(
            "bounded runnable queue selection",
            "no runnable task exists in the bounded queue",
        ))
    }
}

const QUEUE_TASK_A_BYTES: [u8; 19] = [
    0x49,
    0xbc,
    0x11,
    0x11,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0xc6,
    0x44,
    0x24,
    0xf8,
    b'a',
    0xcd,
    TASK_BLOCK_VECTOR,
    0xcd,
    0x81,
];

const QUEUE_TASK_B_BYTES: [u8; 21] = [
    0x49,
    0x81,
    0xfc,
    0x22,
    0x22,
    0x00,
    0x00,
    0x75,
    0x0a,
    0xc6,
    0x44,
    0x24,
    0xf8,
    b'b',
    0x49,
    0xff,
    0xc4,
    0xcd,
    TASK_WAKE_ARM_VECTOR,
    0xcd,
    0x81,
];

const QUEUE_PIC_SETUP_AFTER_CLI: [u8; 36] = [
    0xb0, 0x11, 0xe6, 0x20, 0xe6, 0xa0, 0xb0, 0x40, 0xe6, 0x21, 0xb0, 0x48, 0xe6, 0xa1, 0xb0, 0x04,
    0xe6, 0x21, 0xb0, 0x02, 0xe6, 0xa1, 0xb0, 0x01, 0xe6, 0x21, 0xe6, 0xa1, 0xb0, 0xfe, 0xe6, 0x21,
    0xb0, 0xff, 0xe6, 0xa1,
];

const QUEUE_BLOCK_HANDLER_BYTES: [u8; 17] = [
    0xc6,
    0x04,
    0x25,
    0xc0,
    0x00,
    0x03,
    0x00,
    TaskRunState::Blocked as u8,
    0xb0,
    b'K',
    0xe6,
    0xe9,
    0xe9,
    0xef,
    0x3f,
    0x00,
    0x00,
];

const QUEUE_WAKE_ARM_HANDLER_BYTES: [u8; 26] = [
    0xb0,
    b'P',
    0xe6,
    0xe9,
    0xfb,
    0xf4,
    0x80,
    0x3c,
    0x25,
    0xc0,
    0x00,
    0x03,
    0x00,
    TaskRunState::Runnable as u8,
    0x75,
    0x05,
    0xe9,
    0xeb,
    0x1f,
    0x00,
    0x00,
    0xb0,
    TASK_QUEUE_FAILURE_BYTE,
    0xe6,
    0xe9,
    0xf4,
];

const QUEUE_WAKE_TIMER_HANDLER_BYTES: [u8; 18] = [
    0xc6,
    0x04,
    0x25,
    0xc0,
    0x00,
    0x03,
    0x00,
    TaskRunState::Runnable as u8,
    0xb0,
    b'W',
    0xe6,
    0xe9,
    0xb0,
    0x20,
    0xe6,
    0x20,
    0x48,
    0xcf,
];

const QUEUE_HANDLER_BYTES: [u8; 233] = [
    0x80, 0x3c, 0x25, 0xc8, 0x00, 0x03, 0x00, 0x00, 0x0f, 0x85, 0xd6, 0x00, 0x00, 0x00, 0x80, 0x3c,
    0x25, 0xc9, 0x00, 0x03, 0x00, 0x01, 0x0f, 0x85, 0xc8, 0x00, 0x00, 0x00, 0x80, 0x3c, 0x25, 0xca,
    0x00, 0x03, 0x00, 0x00, 0x0f, 0x84, 0x2f, 0x00, 0x00, 0x00, 0x80, 0x3c, 0x25, 0xca, 0x00, 0x03,
    0x00, 0x01, 0x0f, 0x85, 0xac, 0x00, 0x00, 0x00, 0x80, 0x3c, 0x25, 0xc1, 0x00, 0x03, 0x00, 0x01,
    0x0f, 0x84, 0x75, 0x00, 0x00, 0x00, 0x80, 0x3c, 0x25, 0xc0, 0x00, 0x03, 0x00, 0x01, 0x0f, 0x84,
    0x26, 0x00, 0x00, 0x00, 0xe9, 0x8b, 0x00, 0x00, 0x00, 0x80, 0x3c, 0x25, 0xc0, 0x00, 0x03, 0x00,
    0x01, 0x0f, 0x84, 0x2b, 0x00, 0x00, 0x00, 0x80, 0x3c, 0x25, 0xc1, 0x00, 0x03, 0x00, 0x01, 0x0f,
    0x84, 0x11, 0x00, 0x00, 0x00, 0xe9, 0x6a, 0x00, 0x00, 0x00, 0xfe, 0x04, 0x25, 0xcc, 0x00, 0x03,
    0x00, 0xe9, 0x0c, 0x00, 0x00, 0x00, 0xfe, 0x04, 0x25, 0xcc, 0x00, 0x03, 0x00, 0xe9, 0x29, 0x00,
    0x00, 0x00, 0xc6, 0x04, 0x25, 0xcb, 0x00, 0x03, 0x00, 0x00, 0xc6, 0x04, 0x25, 0xca, 0x00, 0x03,
    0x00, 0x01, 0x0f, 0x20, 0xdb, 0x48, 0x81, 0xfb, 0x00, 0xb0, 0x00, 0x00, 0x0f, 0x85, 0x32, 0x00,
    0x00, 0x00, 0xb0, 0x30, 0xe6, 0xe9, 0xe9, 0x45, 0xcf, 0xff, 0xff, 0xc6, 0x04, 0x25, 0xcb, 0x00,
    0x03, 0x00, 0x01, 0xc6, 0x04, 0x25, 0xca, 0x00, 0x03, 0x00, 0x00, 0x0f, 0x20, 0xdb, 0x48, 0x81,
    0xfb, 0x00, 0x10, 0x00, 0x00, 0x0f, 0x85, 0x09, 0x00, 0x00, 0x00, 0xb0, 0x31, 0xe6, 0xe9, 0xe9,
    0x1c, 0xcf, 0xff, 0xff, 0xb0, 0x46, 0xe6, 0xe9, 0xf4,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnableQueueGuestResult {
    gsi: u32,
    vector: u8,
    lapic_spiv: u32,
    lapic_lint0: u32,
    armed_rflags: u64,
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

impl RunnableQueueGuestResult {
    #[must_use]
    pub const fn gsi(&self) -> u32 {
        self.gsi
    }
    #[must_use]
    pub const fn vector(&self) -> u8 {
        self.vector
    }
    #[must_use]
    pub const fn lapic_spiv(&self) -> u32 {
        self.lapic_spiv
    }
    #[must_use]
    pub const fn lapic_lint0(&self) -> u32 {
        self.lapic_lint0
    }
    #[must_use]
    pub const fn armed_rflags(&self) -> u64 {
        self.armed_rflags
    }
    #[must_use]
    pub const fn first_selection(&self) -> RunnableQueueSnapshot {
        self.first_selection
    }
    #[must_use]
    pub const fn second_selection(&self) -> RunnableQueueSnapshot {
        self.second_selection
    }
    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }
    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
    #[must_use]
    pub const fn task_a(&self) -> TaskContextSnapshot {
        self.task_a
    }
    #[must_use]
    pub const fn task_b(&self) -> TaskContextSnapshot {
        self.task_b
    }
    #[must_use]
    pub const fn terminal(&self) -> TaskTerminalObservation {
        self.terminal
    }
    #[must_use]
    pub const fn final_cr3(&self) -> u64 {
        self.final_cr3
    }
    #[must_use]
    pub const fn final_r12(&self) -> u64 {
        self.final_r12
    }
    #[must_use]
    pub const fn task_a_stack_marker(&self) -> u8 {
        self.task_a_stack_marker
    }
    #[must_use]
    pub const fn task_b_stack_marker(&self) -> u8 {
        self.task_b_stack_marker
    }
    #[must_use]
    pub const fn first_context_pte(&self) -> u64 {
        self.first_context_pte
    }
    #[must_use]
    pub const fn second_context_pte(&self) -> u64 {
        self.second_context_pte
    }
}

pub fn run_bounded_runnable_queue_guest(
    config: VmConfig,
) -> Result<RunnableQueueGuestResult, Error> {
    let kernel_bytes = queue_kernel_bytes();
    let kernel = FlatGuestImage::new(
        PRIVILEGE_KERNEL_ENTRY,
        PRIVILEGE_KERNEL_ENTRY,
        &kernel_bytes,
    )?;
    let task_a = FlatGuestImage::new(
        PRIVILEGE_USER_ENTRY,
        PRIVILEGE_USER_ENTRY,
        &QUEUE_TASK_A_BYTES,
    )?;
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
        &QUEUE_BLOCK_HANDLER_BYTES,
    )?;
    let arm_handler = FlatGuestImage::new(
        TASK_WAKE_ARM_HANDLER,
        TASK_WAKE_ARM_HANDLER,
        &QUEUE_WAKE_ARM_HANDLER_BYTES,
    )?;
    let wake_handler = FlatGuestImage::new(
        TASK_WAKE_TIMER_HANDLER,
        TASK_WAKE_TIMER_HANDLER,
        &QUEUE_WAKE_TIMER_HANDLER_BYTES,
    )?;
    let queue_handler =
        FlatGuestImage::new(TASK_QUEUE_HANDLER, TASK_QUEUE_HANDLER, &QUEUE_HANDLER_BYTES)?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = AddressSpaceSwitchLayout::new(memory.region())
        .expect("fixed bounded runnable-queue layout remains valid");
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
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(layout.privilege_layout())?;
    let lapic = vcpu.configure_legacy_pic_extint()?;
    let mut port_io = PortIoBus::with_debug_port();

    let blocked_io = run_queue_debug_output(&mut vcpu, &mut port_io, b'K', "queue block A")?;
    let blocked_state = read_queue_state(
        vm.guest_memory()
            .expect("registered runnable-queue memory remains VM-owned"),
        TASK_QUEUE_A_STATE_ADDR,
    )?;
    if blocked_state != TaskRunState::Blocked {
        return Err(verification_error(
            "bounded runnable queue block state",
            format!("expected A Blocked after K, got {blocked_state:?}"),
        ));
    }

    let first_select_io = run_queue_debug_output(
        &mut vcpu,
        &mut port_io,
        b'1',
        "queue select B after blocked A",
    )?;
    let first_selection = read_queue_snapshot(
        vm.guest_memory()
            .expect("registered runnable-queue memory remains VM-owned"),
    )?;
    require_first_queue_selection(first_selection)?;

    let scheduler_a_io = run_queue_debug_output(
        &mut vcpu,
        &mut port_io,
        b'A',
        "queue-selected B scheduler handoff",
    )?;
    let armed_io = run_queue_debug_output(&mut vcpu, &mut port_io, b'P', "queue wake arm barrier")?;
    let armed = vcpu.registers()?;
    require_queue_interrupt_disabled_flags("queue wake arm state", armed.rflags)?;

    let timer_irq = vm.duplicate_irq_line_handle().map_err(|source| {
        queue_vm_error("duplicate runnable-queue wake IRQ-line handle", source)
    })?;
    let watchdog_irq = vm
        .duplicate_irq_line_handle()
        .map_err(|source| queue_vm_error("duplicate runnable-queue watchdog handle", source))?;
    timer_irq
        .set_gsi_level(TASK_WAKE_GSI, false)
        .map_err(|source| queue_vm_error("preflight runnable-queue IRQ line", source))?;
    watchdog_irq
        .set_gsi_level(TASK_WAKE_GSI, false)
        .map_err(|source| queue_vm_error("preflight runnable-queue watchdog line", source))?;

    let timer_worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(
            TASK_QUEUE_TIMER_DELAY_MILLIS,
        ));
        timer_irq.pulse_gsi_edge(TASK_WAKE_GSI)
    });
    let (watchdog_cancel_tx, watchdog_cancel_rx) = std::sync::mpsc::channel::<()>();
    let watchdog_worker = std::thread::spawn(move || -> io::Result<bool> {
        match watchdog_cancel_rx
            .recv_timeout(std::time::Duration::from_secs(TASK_QUEUE_WATCHDOG_SECONDS))
        {
            Ok(()) => Ok(false),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                watchdog_irq.pulse_gsi_edge(TASK_WAKE_GSI)?;
                Ok(true)
            }
        }
    });

    let execution = (|| -> Result<
        (
            PortIoExit,
            PortIoExit,
            RunnableQueueSnapshot,
            PortIoExit,
            PortIoExit,
            PortIoExit,
        ),
        Error,
    > {
        let wake_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'W',
            "queue external wake handler",
        )?;
        let second_select_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'0',
            "queue select woken A",
        )?;
        let second_selection = read_queue_snapshot(
            vm.guest_memory()
                .expect("registered runnable-queue memory remains VM-owned"),
        )?;
        require_second_queue_selection(second_selection)?;
        let scheduler_b_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'B',
            "queue-selected A scheduler handoff",
        )?;
        let restored_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'R',
            "runnable-queue restored A observation",
        )?;
        let done_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'D',
            "runnable-queue completion barrier",
        )?;
        Ok((
            wake_io,
            second_select_io,
            second_selection,
            scheduler_b_io,
            restored_io,
            done_io,
        ))
    })();

    let _ = watchdog_cancel_tx.send(());
    let timer_result = timer_worker.join().map_err(|_| {
        verification_error(
            "join runnable-queue timer worker",
            "runnable-queue timer worker panicked before reporting its GSI pulse",
        )
    })?;
    let watchdog_fired = watchdog_worker
        .join()
        .map_err(|_| {
            verification_error(
                "join runnable-queue watchdog",
                "runnable-queue watchdog panicked before reporting whether it fired",
            )
        })?
        .map_err(|source| queue_vm_error("runnable-queue watchdog GSI", source))?;
    timer_result.map_err(|source| queue_vm_error("runnable-queue wake GSI pulse", source))?;
    if watchdog_fired {
        return Err(verification_error(
            "runnable-queue watchdog",
            "watchdog injected fallback GSI; queue wakeup was not independently proven",
        ));
    }

    let (wake_io, second_select_io, second_selection, scheduler_b_io, restored_io, done_io) =
        execution?;
    let io_exits = vec![
        blocked_io,
        first_select_io,
        scheduler_a_io,
        armed_io,
        wake_io,
        second_select_io,
        scheduler_b_io,
        restored_io,
        done_io,
    ];
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != TASK_RUNNABLE_QUEUE_PROOF
        || io_exits.len() != TASK_RUNNABLE_QUEUE_PROOF.len()
    {
        return Err(verification_error(
            "bounded runnable queue proof",
            format!(
                "expected {:?} across {} exits, got {:?} across {} exits",
                TASK_RUNNABLE_QUEUE_PROOF,
                TASK_RUNNABLE_QUEUE_PROOF.len(),
                proof,
                io_exits.len()
            ),
        ));
    }

    let final_regs = vcpu.capture_register_snapshot()?;
    let final_special = vcpu.capture_special_register_snapshot()?;
    let guest_memory = vm
        .guest_memory()
        .expect("registered runnable-queue memory remains VM-owned");
    let task_a_context = read_context(guest_memory, TASK_A_CONTEXT_ADDR)?;
    let task_b_context = read_context(guest_memory, TASK_B_CONTEXT_ADDR)?;
    let terminal_observation = read_terminal_observation(guest_memory)?;
    let task_a_stack_marker = read_byte(guest_memory, TASK_A_STACK_MARKER_PHYS)?;
    let task_b_stack_marker = read_byte(guest_memory, TASK_B_STACK_MARKER_PHYS)?;
    let first_context_pte = read_pte(
        guest_memory,
        PRIVILEGE_PT_ADDR,
        TASK_CONTEXT_PAGE_ADDR.get(),
    )?;
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

    Ok(RunnableQueueGuestResult {
        gsi: TASK_WAKE_GSI,
        vector: TASK_WAKE_TIMER_VECTOR,
        lapic_spiv: lapic.spiv(),
        lapic_lint0: lapic.lint0(),
        armed_rflags: armed.rflags,
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
    })
}

fn queue_kernel_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(KERNEL_BOOT_BYTES.len() + QUEUE_PIC_SETUP_AFTER_CLI.len());
    bytes.push(0xfa);
    bytes.extend_from_slice(&QUEUE_PIC_SETUP_AFTER_CLI);
    bytes.extend_from_slice(&KERNEL_BOOT_BYTES[1..]);
    bytes
}

fn initialize_queue_metadata(memory: &mut GuestMemory) -> Result<(), Error> {
    let model = QueueModel::new([RunnableTaskId::A, RunnableTaskId::B])?;
    memory.write(TASK_QUEUE_A_STATE_ADDR, &[TaskRunState::Runnable as u8])?;
    memory.write(TASK_QUEUE_B_STATE_ADDR, &[TaskRunState::Runnable as u8])?;
    memory.write(TASK_QUEUE_ENTRY0_ADDR, &[model.entries[0] as u8])?;
    memory.write(TASK_QUEUE_ENTRY1_ADDR, &[model.entries[1] as u8])?;
    memory.write(TASK_QUEUE_HEAD_ADDR, &[model.head as u8])?;
    memory.write(TASK_QUEUE_SELECTED_ADDR, &[0xff])?;
    memory.write(TASK_QUEUE_SKIP_COUNT_ADDR, &[0])?;
    Ok(())
}

fn install_queue_gates(
    memory: &mut GuestMemory,
    layout: &AddressSpaceSwitchLayout,
) -> Result<(), Error> {
    for (vector, handler, user_callable) in [
        (TASK_WAKE_TIMER_VECTOR, TASK_WAKE_TIMER_HANDLER, false),
        (TASK_BLOCK_VECTOR, TASK_BLOCK_HANDLER, true),
        (TASK_WAKE_ARM_VECTOR, TASK_WAKE_ARM_HANDLER, true),
    ] {
        let end = u16::from(vector)
            .checked_add(1)
            .and_then(|entries| entries.checked_mul(X86_INTERRUPT_GATE_SIZE as u16))
            .and_then(|bytes| bytes.checked_sub(1))
            .expect("8-bit vector always fits a 4 KiB IDT");
        if end > layout.privilege_layout().idt_limit() {
            return Err(verification_error(
                "bounded runnable queue IDT gate",
                format!(
                    "vector {vector:#x} exceeds existing privilege IDT limit {:#x}",
                    layout.privilege_layout().idt_limit()
                ),
            ));
        }
        memory.write(
            GuestPhysAddr::new(
                PRIVILEGE_IDT_ADDR.get() + u64::from(vector) * X86_INTERRUPT_GATE_SIZE,
            ),
            &encode_queue_interrupt_gate(handler.get(), user_callable),
        )?;
    }
    Ok(())
}

fn encode_queue_interrupt_gate(handler: u64, user_callable: bool) -> [u8; 16] {
    let mut gate = [0_u8; 16];
    gate[0..2].copy_from_slice(&(handler as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&X86_KERNEL_CODE_SELECTOR.to_le_bytes());
    gate[4] = 0;
    gate[5] = if user_callable {
        X86_RING3_INTERRUPT_GATE
    } else {
        X86_RING0_INTERRUPT_GATE
    };
    gate[6..8].copy_from_slice(&((handler >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((handler >> 32) as u32).to_le_bytes());
    gate
}

fn run_queue_debug_output(
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    expected: u8,
    stage: &'static str,
) -> Result<PortIoExit, Error> {
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Io {
        return Err(Error::VmExit(
            crate::error::VmExitError::UnexpectedSequence {
                stage,
                expected_reason: VcpuExit::Io.reason(),
                actual_reason: exit.reason(),
            },
        ));
    }
    let io_exit = vcpu.port_io_exit()?;
    if io_exit.direction() != PortIoDirection::Out
        || io_exit.size() != 1
        || io_exit.port() != DEBUG_PORT
        || io_exit.count() != 1
        || io_exit.output_data() != [expected]
    {
        return Err(verification_error(
            stage,
            format!("unexpected I/O exit {io_exit:?}, expected debug byte {expected:#x}"),
        ));
    }
    if port_io.dispatch(&io_exit)? != PortIoService::Output {
        return Err(verification_error(
            stage,
            "debug output unexpectedly requested an input response",
        ));
    }
    Ok(io_exit)
}

fn read_queue_state(memory: &GuestMemory, address: GuestPhysAddr) -> Result<TaskRunState, Error> {
    match read_byte(memory, address)? {
        0 => Ok(TaskRunState::Blocked),
        1 => Ok(TaskRunState::Runnable),
        value => Err(verification_error(
            "bounded runnable queue state encoding",
            format!("unexpected task run-state value {value:#x}"),
        )),
    }
}

fn read_queue_task_id(
    memory: &GuestMemory,
    address: GuestPhysAddr,
) -> Result<RunnableTaskId, Error> {
    match read_byte(memory, address)? {
        0 => Ok(RunnableTaskId::A),
        1 => Ok(RunnableTaskId::B),
        value => Err(verification_error(
            "bounded runnable queue task-id encoding",
            format!("unexpected task ID {value:#x}"),
        )),
    }
}

fn read_queue_snapshot(memory: &GuestMemory) -> Result<RunnableQueueSnapshot, Error> {
    Ok(RunnableQueueSnapshot {
        entry0: read_queue_task_id(memory, TASK_QUEUE_ENTRY0_ADDR)?,
        entry1: read_queue_task_id(memory, TASK_QUEUE_ENTRY1_ADDR)?,
        head: read_byte(memory, TASK_QUEUE_HEAD_ADDR)?,
        selected: read_queue_task_id(memory, TASK_QUEUE_SELECTED_ADDR)?,
        skip_count: read_byte(memory, TASK_QUEUE_SKIP_COUNT_ADDR)?,
        task_a_state: read_queue_state(memory, TASK_QUEUE_A_STATE_ADDR)?,
        task_b_state: read_queue_state(memory, TASK_QUEUE_B_STATE_ADDR)?,
    })
}

fn require_first_queue_selection(snapshot: RunnableQueueSnapshot) -> Result<(), Error> {
    let mut model = QueueModel::new([RunnableTaskId::A, RunnableTaskId::B])?;
    let expected = model.select_next(TaskRunState::Blocked, TaskRunState::Runnable)?;
    if snapshot.entry0 != RunnableTaskId::A
        || snapshot.entry1 != RunnableTaskId::B
        || snapshot.head != model.head as u8
        || snapshot.selected != expected.task
        || snapshot.skip_count != expected.skipped
        || snapshot.task_a_state != TaskRunState::Blocked
        || snapshot.task_b_state != TaskRunState::Runnable
    {
        return Err(verification_error(
            "bounded runnable queue first selection",
            format!("unexpected first queue snapshot {snapshot:?}"),
        ));
    }
    Ok(())
}

fn require_second_queue_selection(snapshot: RunnableQueueSnapshot) -> Result<(), Error> {
    let mut model = QueueModel::new([RunnableTaskId::A, RunnableTaskId::B])?;
    let first = model.select_next(TaskRunState::Blocked, TaskRunState::Runnable)?;
    let second = model.select_next(TaskRunState::Runnable, TaskRunState::Runnable)?;
    let expected_skips = first
        .skipped
        .checked_add(second.skipped)
        .expect("two bounded selections fit u8");
    if snapshot.entry0 != RunnableTaskId::A
        || snapshot.entry1 != RunnableTaskId::B
        || snapshot.head != model.head as u8
        || snapshot.selected != second.task
        || snapshot.skip_count != expected_skips
        || snapshot.task_a_state != TaskRunState::Runnable
        || snapshot.task_b_state != TaskRunState::Runnable
    {
        return Err(verification_error(
            "bounded runnable queue second selection",
            format!("unexpected second queue snapshot {snapshot:?}"),
        ));
    }
    Ok(())
}

fn require_queue_interrupt_disabled_flags(stage: &'static str, rflags: u64) -> Result<(), Error> {
    if rflags & 0x2 != 0x2 || rflags & X86_RFLAGS_INTERRUPT_ENABLE != 0 {
        return Err(verification_error(
            stage,
            format!("expected architectural bit1 set and IF clear, got RFLAGS {rflags:#x}"),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_queue_context_state(
    task_a: TaskContextSnapshot,
    task_b: TaskContextSnapshot,
    terminal: TaskTerminalObservation,
    final_cr3: u64,
    final_r12: u64,
    task_a_stack_marker: u8,
    task_b_stack_marker: u8,
    first_context_pte: u64,
    second_context_pte: u64,
) -> Result<(), Error> {
    let expected_a = TaskContextSnapshot {
        cr3: ADDRESS_SPACE_A_CR3.get(),
        rip: TASK_A_SAVED_RIP,
        rsp: TASK_A_INITIAL_RSP,
        rflags: TASK_USER_RFLAGS,
        r12: TASK_A_R12,
        save_count: 1,
    };
    let expected_b = TaskContextSnapshot {
        cr3: ADDRESS_SPACE_B_PML4_ADDR.get(),
        rip: TASK_B_SAVED_RIP,
        rsp: TASK_B_INITIAL_RSP,
        rflags: TASK_USER_RFLAGS,
        r12: TASK_B_SAVED_R12,
        save_count: 1,
    };
    let expected_terminal = TaskTerminalObservation {
        cr3: ADDRESS_SPACE_A_CR3.get(),
        rip: TASK_TERMINAL_USER_RIP,
        rsp: TASK_A_INITIAL_RSP,
        r12: TASK_A_R12,
    };
    if task_a != expected_a || task_b != expected_b || terminal != expected_terminal {
        return Err(verification_error(
            "bounded runnable queue context ownership",
            format!(
                "expected A {expected_a:?}, B {expected_b:?}, terminal {expected_terminal:?}; got A {task_a:?}, B {task_b:?}, terminal {terminal:?}"
            ),
        ));
    }
    if final_cr3 != ADDRESS_SPACE_A_CR3.get() || final_r12 != TASK_A_R12 {
        return Err(verification_error(
            "bounded runnable queue final registers",
            format!(
                "expected CR3={:#x} R12={:#x}, got CR3={final_cr3:#x} R12={final_r12:#x}",
                ADDRESS_SPACE_A_CR3.get(),
                TASK_A_R12
            ),
        ));
    }
    if task_a_stack_marker != b'a' || task_b_stack_marker != b'b' {
        return Err(verification_error(
            "bounded runnable queue stack ownership",
            format!(
                "expected physical stack markers a/b, got {task_a_stack_marker:#x}/{task_b_stack_marker:#x}"
            ),
        ));
    }
    for (role, pte) in [
        ("A task-context page", first_context_pte),
        ("B task-context page", second_context_pte),
    ] {
        if pte & X86_PAGE_ADDRESS_MASK != TASK_CONTEXT_PAGE_ADDR.get()
            || pte & (X86_PAGE_PRESENT | X86_PAGE_WRITABLE)
                != (X86_PAGE_PRESENT | X86_PAGE_WRITABLE)
            || pte & X86_PAGE_USER != 0
        {
            return Err(verification_error(
                "bounded runnable queue context PTE",
                format!(
                    "{role}: expected supervisor P/W mapping to {:#x}, got {pte:#x}",
                    TASK_CONTEXT_PAGE_ADDR.get()
                ),
            ));
        }
    }
    Ok(())
}

fn queue_vm_error(operation: &'static str, source: io::Error) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VmOperation { operation, source })
}

const _: () = {
    assert!(TASK_QUEUE_HANDLER.get() < LONG_MODE_IDENTITY_MAP_SIZE);
    assert!(TASK_QUEUE_A_STATE_ADDR.get() >= TASK_CONTEXT_PAGE_ADDR.get());
    assert!(TASK_QUEUE_SKIP_COUNT_ADDR.get() < TASK_CONTEXT_PAGE_ADDR.get() + LONG_MODE_PAGE_SIZE);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_model_skips_blocked_entry_then_selects_woken_task() {
        let mut queue = QueueModel::new([RunnableTaskId::A, RunnableTaskId::B]).unwrap();
        let first = queue
            .select_next(TaskRunState::Blocked, TaskRunState::Runnable)
            .unwrap();
        assert_eq!(first.task, RunnableTaskId::B);
        assert_eq!(first.skipped, 1);
        assert_eq!(queue.head, 0);

        let second = queue
            .select_next(TaskRunState::Runnable, TaskRunState::Runnable)
            .unwrap();
        assert_eq!(second.task, RunnableTaskId::A);
        assert_eq!(second.skipped, 0);
        assert_eq!(queue.head, 1);
    }

    #[test]
    fn queue_model_rejects_duplicates_and_no_runnable_task() {
        assert!(QueueModel::new([RunnableTaskId::A, RunnableTaskId::A]).is_err());
        let mut queue = QueueModel::new([RunnableTaskId::A, RunnableTaskId::B]).unwrap();
        assert!(queue
            .select_next(TaskRunState::Blocked, TaskRunState::Blocked)
            .is_err());
    }

    #[test]
    fn queue_handlers_preserve_block_wake_and_selection_contract() {
        assert_eq!(
            &QUEUE_BLOCK_HANDLER_BYTES[..12],
            &[0xc6, 0x04, 0x25, 0xc0, 0x00, 0x03, 0x00, 0, 0xb0, b'K', 0xe6, 0xe9]
        );
        assert_eq!(&QUEUE_WAKE_ARM_HANDLER_BYTES[4..6], &[0xfb, 0xf4]);
        assert_eq!(
            &QUEUE_WAKE_TIMER_HANDLER_BYTES[8..12],
            &[0xb0, b'W', 0xe6, 0xe9]
        );
        assert_eq!(QUEUE_HANDLER_BYTES.len(), 233);
        assert!(QUEUE_HANDLER_BYTES
            .windows(4)
            .any(|window| window == [0xb0, b'1', 0xe6, 0xe9]));
        assert!(QUEUE_HANDLER_BYTES
            .windows(4)
            .any(|window| window == [0xb0, b'0', 0xe6, 0xe9]));
    }
}
