use super::*;
use crate::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use crate::portio::PortIoService;
use crate::privilege::PRIVILEGE_IDT_ADDR;
use crate::vcpu::Vcpu;

pub const TASK_PREEMPTION_GSI: u32 = 0;
pub const TASK_PREEMPTION_TIMER_VECTOR: u8 = 0x40;
pub const TASK_PREEMPTION_ARM_VECTOR: u8 = 0x7f;
pub const TASK_PREEMPTION_ARM_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x14000);
pub const TASK_PREEMPTION_TIMER_WRAPPER: GuestPhysAddr = GuestPhysAddr::new(0x16000);
pub const TASK_PREEMPTION_PROOF: &[u8; 5] = b"PABRD";

const TASK_PREEMPTION_TIMER_DELAY_MILLIS: u64 = 10;
const TASK_PREEMPTION_WATCHDOG_SECONDS: u64 = 5;
const TASK_PREEMPTION_ARM_BYTE: u8 = b'P';
const TASK_PREEMPTION_FAILURE_BYTE: u8 = b'F';
const X86_INTERRUPT_GATE_SIZE: u64 = 16;
const X86_KERNEL_CODE_SELECTOR: u16 = 0x08;
const X86_RING0_INTERRUPT_GATE: u8 = 0x8e;
const X86_RING3_INTERRUPT_GATE: u8 = 0xee;

const PIC_SETUP_AFTER_CLI: [u8; 36] = [
    0xb0, 0x11, 0xe6, 0x20, 0xe6, 0xa0, // ICW1: initialize master/slave
    0xb0, 0x40, 0xe6, 0x21, // master IRQ0..7 -> vector 0x40..0x47
    0xb0, 0x48, 0xe6, 0xa1, // slave IRQ8..15 -> vector 0x48..0x4f
    0xb0, 0x04, 0xe6, 0x21, // master cascade on IRQ2
    0xb0, 0x02, 0xe6, 0xa1, // slave cascade identity
    0xb0, 0x01, 0xe6, 0x21, 0xe6, 0xa1, // 8086 mode
    0xb0, 0xfe, 0xe6, 0x21, // unmask only IRQ0
    0xb0, 0xff, 0xe6, 0xa1, // mask all slave IRQs
];

const PREEMPT_TASK_A_BYTES: [u8; 19] = [
    0x49,
    0xbc,
    0x11,
    0x11,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00, // mov r12,0x1111
    0xc6,
    0x44,
    0x24,
    0xf8,
    b'a', // mov byte [rsp-8],'a'
    0xcd,
    TASK_PREEMPTION_ARM_VECTOR, // int 0x7f: deterministic arm rendezvous
    0xcd,
    0x81, // terminal only after the preempted A context is restored
];

const PREEMPT_ARM_HANDLER_BYTES: [u8; 11] = [
    0xb0,
    TASK_PREEMPTION_ARM_BYTE,
    0xe6,
    0xe9, // host-visible arm barrier under IF=0
    0xfb, // sti
    0xf4, // hlt: STI shadow closes the early-edge race
    0xb0,
    TASK_PREEMPTION_FAILURE_BYTE,
    0xe6,
    0xe9, // must never resume directly
    0xf4,
];

const PREEMPT_TIMER_WRAPPER_BYTES: [u8; 13] = [
    0xb0, 0x20, 0xe6, 0x20, // EOI master PIC before leaving the timer path
    0x48, 0x83, 0xc4, 0x18, // discard timer's CPL0 RIP/CS/RFLAGS frame
    0xe9, 0xf3, 0xef, 0xff, 0xff, // jmp 0x15000 existing scheduler from 0x1600d
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerTaskPreemptionGuestResult {
    gsi: u32,
    vector: u8,
    lapic_spiv: u32,
    lapic_lint0: u32,
    armed_rflags: u64,
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

impl TimerTaskPreemptionGuestResult {
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

pub fn run_timer_task_preemption_guest(
    config: VmConfig,
) -> Result<TimerTaskPreemptionGuestResult, Error> {
    let kernel_bytes = preemption_kernel_bytes();
    let kernel = FlatGuestImage::new(
        PRIVILEGE_KERNEL_ENTRY,
        PRIVILEGE_KERNEL_ENTRY,
        &kernel_bytes,
    )?;
    let task_a = FlatGuestImage::new(
        PRIVILEGE_USER_ENTRY,
        PRIVILEGE_USER_ENTRY,
        &PREEMPT_TASK_A_BYTES,
    )?;
    let task_b = FlatGuestImage::new(
        ADDRESS_SPACE_B_USER_CODE_BACKING,
        ADDRESS_SPACE_B_USER_CODE_BACKING,
        &TASK_B_BYTES,
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
    let arm_handler = FlatGuestImage::new(
        TASK_PREEMPTION_ARM_HANDLER,
        TASK_PREEMPTION_ARM_HANDLER,
        &PREEMPT_ARM_HANDLER_BYTES,
    )?;
    let timer_wrapper = FlatGuestImage::new(
        TASK_PREEMPTION_TIMER_WRAPPER,
        TASK_PREEMPTION_TIMER_WRAPPER,
        &PREEMPT_TIMER_WRAPPER_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = AddressSpaceSwitchLayout::new(memory.region())
        .expect("fixed bounded timer-preemption task layout remains valid");
    layout.install_tables(&mut memory)?;
    install_preemption_gates(&mut memory, &layout)?;
    kernel.load(&mut memory)?;
    task_a.load(&mut memory)?;
    task_b.load(&mut memory)?;
    scheduler.load(&mut memory)?;
    terminal.load(&mut memory)?;
    arm_handler.load(&mut memory)?;
    timer_wrapper.load(&mut memory)?;
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
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(layout.privilege_layout())?;
    let lapic = vcpu.configure_legacy_pic_extint()?;
    let mut port_io = PortIoBus::with_debug_port();

    let armed_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        TASK_PREEMPTION_ARM_BYTE,
        "timer task preemption arm barrier",
    )?;
    let armed = vcpu.registers()?;
    require_interrupt_disabled_flags("timer task preemption arm state", armed.rflags)?;

    let timer_irq = vm
        .duplicate_irq_line_handle()
        .map_err(|source| timer_vm_error("duplicate timer-preemption IRQ-line handle", source))?;
    let watchdog_irq = vm
        .duplicate_irq_line_handle()
        .map_err(|source| timer_vm_error("duplicate timer-preemption watchdog handle", source))?;
    timer_irq
        .set_gsi_level(TASK_PREEMPTION_GSI, false)
        .map_err(|source| timer_vm_error("preflight timer-preemption IRQ line", source))?;
    watchdog_irq
        .set_gsi_level(TASK_PREEMPTION_GSI, false)
        .map_err(|source| timer_vm_error("preflight timer-preemption watchdog line", source))?;

    let timer_worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(
            TASK_PREEMPTION_TIMER_DELAY_MILLIS,
        ));
        timer_irq.pulse_gsi_edge(TASK_PREEMPTION_GSI)
    });
    let (watchdog_cancel_tx, watchdog_cancel_rx) = std::sync::mpsc::channel::<()>();
    let watchdog_worker = std::thread::spawn(move || -> io::Result<bool> {
        match watchdog_cancel_rx.recv_timeout(std::time::Duration::from_secs(
            TASK_PREEMPTION_WATCHDOG_SECONDS,
        )) {
            Ok(()) => Ok(false),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                watchdog_irq.pulse_gsi_edge(TASK_PREEMPTION_GSI)?;
                Ok(true)
            }
        }
    });

    let execution = (|| -> Result<Vec<PortIoExit>, Error> {
        let mut exits = vec![armed_io];
        for (byte, stage) in [
            (b'A', "timer-preempted task A scheduler entry"),
            (b'B', "task B cooperative scheduler entry"),
            (b'R', "restored task A terminal observation"),
            (b'D', "timer task preemption completion barrier"),
        ] {
            exits.push(run_expected_debug_output(
                &mut vcpu,
                &mut port_io,
                byte,
                stage,
            )?);
        }
        Ok(exits)
    })();

    let _ = watchdog_cancel_tx.send(());
    let timer_result = timer_worker.join().map_err(|_| {
        verification_error(
            "join timer-preemption worker",
            "timer-preemption worker panicked before reporting its GSI pulse",
        )
    })?;
    let watchdog_fired = watchdog_worker
        .join()
        .map_err(|_| {
            verification_error(
                "join timer-preemption watchdog",
                "timer-preemption watchdog panicked before reporting whether it fired",
            )
        })?
        .map_err(|source| timer_vm_error("timer-preemption watchdog GSI", source))?;
    timer_result.map_err(|source| timer_vm_error("timer-preemption GSI pulse", source))?;
    if watchdog_fired {
        return Err(verification_error(
            "timer task preemption watchdog",
            "watchdog injected fallback GSI; timer-driven context switching was not independently proven",
        ));
    }
    let io_exits = execution?;
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != TASK_PREEMPTION_PROOF || io_exits.len() != TASK_PREEMPTION_PROOF.len() {
        return Err(verification_error(
            "timer task preemption proof",
            format!(
                "expected {:?} across {} exits, got {:?} across {} exits",
                TASK_PREEMPTION_PROOF,
                TASK_PREEMPTION_PROOF.len(),
                proof,
                io_exits.len()
            ),
        ));
    }

    let final_regs = vcpu.capture_register_snapshot()?;
    let final_special = vcpu.capture_special_register_snapshot()?;
    let guest_memory = vm
        .guest_memory()
        .expect("registered timer-preemption task memory remains VM-owned");
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

    validate_preemption_state(
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

    Ok(TimerTaskPreemptionGuestResult {
        gsi: TASK_PREEMPTION_GSI,
        vector: TASK_PREEMPTION_TIMER_VECTOR,
        lapic_spiv: lapic.spiv(),
        lapic_lint0: lapic.lint0(),
        armed_rflags: armed.rflags,
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

fn preemption_kernel_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(KERNEL_BOOT_BYTES.len() + PIC_SETUP_AFTER_CLI.len());
    bytes.push(0xfa);
    bytes.extend_from_slice(&PIC_SETUP_AFTER_CLI);
    bytes.extend_from_slice(&KERNEL_BOOT_BYTES[1..]);
    bytes
}

fn install_preemption_gates(
    memory: &mut GuestMemory,
    layout: &AddressSpaceSwitchLayout,
) -> Result<(), Error> {
    for (vector, handler, user_callable) in [
        (
            TASK_PREEMPTION_TIMER_VECTOR,
            TASK_PREEMPTION_TIMER_WRAPPER,
            false,
        ),
        (
            TASK_PREEMPTION_ARM_VECTOR,
            TASK_PREEMPTION_ARM_HANDLER,
            true,
        ),
    ] {
        let end = u16::from(vector)
            .checked_add(1)
            .and_then(|entries| entries.checked_mul(X86_INTERRUPT_GATE_SIZE as u16))
            .and_then(|bytes| bytes.checked_sub(1))
            .expect("8-bit vector always fits a 4 KiB IDT");
        if end > layout.privilege_layout().idt_limit() {
            return Err(verification_error(
                "timer task preemption IDT gate",
                format!(
                    "vector {vector:#x} exceeds existing privilege IDT limit {:#x}",
                    layout.privilege_layout().idt_limit()
                ),
            ));
        }
        let gate_address = GuestPhysAddr::new(
            PRIVILEGE_IDT_ADDR.get() + u64::from(vector) * X86_INTERRUPT_GATE_SIZE,
        );
        memory.write(
            gate_address,
            &encode_interrupt_gate(handler.get(), user_callable),
        )?;
    }
    Ok(())
}

fn encode_interrupt_gate(handler: u64, user_callable: bool) -> [u8; 16] {
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

fn run_expected_debug_output(
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

fn require_interrupt_disabled_flags(stage: &'static str, rflags: u64) -> Result<(), Error> {
    if rflags & 0x2 != 0x2 || rflags & X86_RFLAGS_INTERRUPT_ENABLE != 0 {
        return Err(verification_error(
            stage,
            format!("expected architectural bit1 set and IF clear, got RFLAGS {rflags:#x}"),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_preemption_state(
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
            "timer task preemption context ownership",
            format!(
                "expected A {expected_a:?}, B {expected_b:?}, terminal {expected_terminal:?}; got A {task_a:?}, B {task_b:?}, terminal {terminal:?}"
            ),
        ));
    }
    if final_cr3 != ADDRESS_SPACE_A_CR3.get() || final_r12 != TASK_A_R12 {
        return Err(verification_error(
            "timer task preemption final registers",
            format!(
                "expected CR3={:#x} R12={:#x}, got CR3={final_cr3:#x} R12={final_r12:#x}",
                ADDRESS_SPACE_A_CR3.get(),
                TASK_A_R12
            ),
        ));
    }
    if task_a_stack_marker != b'a' || task_b_stack_marker != b'b' {
        return Err(verification_error(
            "timer task preemption stack ownership",
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
                "timer task preemption context PTE",
                format!(
                    "{role}: expected supervisor P/W mapping to {:#x}, got {pte:#x}",
                    TASK_CONTEXT_PAGE_ADDR.get()
                ),
            ));
        }
    }
    Ok(())
}

fn timer_vm_error(operation: &'static str, source: io::Error) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VmOperation { operation, source })
}

const _: () = {
    assert!(TASK_PREEMPTION_TIMER_WRAPPER.get() < LONG_MODE_IDENTITY_MAP_SIZE);
    assert!(TASK_PREEMPTION_ARM_HANDLER.get() < LONG_MODE_IDENTITY_MAP_SIZE);
    assert!(TASK_PREEMPTION_TIMER_VECTOR < TASK_PREEMPTION_ARM_VECTOR);
    assert!(TASK_PREEMPTION_ARM_VECTOR < 0x80);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preemptive_task_a_arms_without_cooperative_scheduler_trap() {
        assert_eq!(PREEMPT_TASK_A_BYTES.len(), TASK_A_BYTES.len());
        assert_eq!(&PREEMPT_TASK_A_BYTES[15..19], &[0xcd, 0x7f, 0xcd, 0x81]);
        assert!(!PREEMPT_TASK_A_BYTES
            .windows(2)
            .any(|window| window == [0xcd, 0x80]));
        assert_eq!(PRIVILEGE_USER_ENTRY.get() + 17, TASK_A_SAVED_RIP);
    }

    #[test]
    fn timer_wrapper_discards_only_nested_kernel_frame_then_reuses_scheduler() {
        assert_eq!(PREEMPT_TIMER_WRAPPER_BYTES.len(), 13);
        assert_eq!(
            &PREEMPT_TIMER_WRAPPER_BYTES[4..8],
            &[0x48, 0x83, 0xc4, 0x18]
        );
        let delta = i32::from_le_bytes(PREEMPT_TIMER_WRAPPER_BYTES[9..13].try_into().unwrap());
        let next = TASK_PREEMPTION_TIMER_WRAPPER.get() + PREEMPT_TIMER_WRAPPER_BYTES.len() as u64;
        assert_eq!(
            (next as i64 + i64::from(delta)) as u64,
            PRIVILEGE_RETURN_HANDLER.get()
        );
    }

    #[test]
    fn additional_gates_keep_timer_kernel_only_and_arm_user_callable() {
        let timer = encode_interrupt_gate(TASK_PREEMPTION_TIMER_WRAPPER.get(), false);
        let arm = encode_interrupt_gate(TASK_PREEMPTION_ARM_HANDLER.get(), true);
        assert_eq!(timer[5], X86_RING0_INTERRUPT_GATE);
        assert_eq!(arm[5], X86_RING3_INTERRUPT_GATE);
        assert_eq!(TASK_PREEMPTION_TIMER_VECTOR, 0x40);
        assert_eq!(TASK_PREEMPTION_ARM_VECTOR, 0x7f);
    }

    #[test]
    fn preemption_kernel_boot_initializes_pic_before_existing_ring3_entry() {
        let bytes = preemption_kernel_bytes();
        assert_eq!(bytes[0], 0xfa);
        assert_eq!(&bytes[1..7], &[0xb0, 0x11, 0xe6, 0x20, 0xe6, 0xa0]);
        assert_eq!(&bytes[33..37], &[0xb0, 0xff, 0xe6, 0xa1]);
        assert_eq!(&bytes[bytes.len() - 2..], &[0x48, 0xcf]);
    }
}
