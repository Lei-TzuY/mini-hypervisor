#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitDirtyStageEvidence {
    dirty: Vec<u64>,
    cleared: Vec<u64>,
}

impl WaitDirtyStageEvidence {
    #[must_use]
    pub fn dirty(&self) -> &[u64] {
        &self.dirty
    }

    #[must_use]
    pub fn cleared(&self) -> &[u64] {
        &self.cleared
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitChannelDirtyDeltaGuestResult {
    wait: WaitChannelGuestResult,
    context_page_index: u64,
    setup_dirty: Vec<u64>,
    clean_baseline: Vec<u64>,
    blocked: WaitDirtyStageEvidence,
    mismatch: WaitDirtyStageEvidence,
    wake: WaitDirtyStageEvidence,
    final_state: WaitDirtyStageEvidence,
}

impl WaitChannelDirtyDeltaGuestResult {
    #[must_use]
    pub const fn wait(&self) -> &WaitChannelGuestResult {
        &self.wait
    }

    #[must_use]
    pub const fn context_page_index(&self) -> u64 {
        self.context_page_index
    }

    #[must_use]
    pub fn setup_dirty(&self) -> &[u64] {
        &self.setup_dirty
    }

    #[must_use]
    pub fn clean_baseline(&self) -> &[u64] {
        &self.clean_baseline
    }

    #[must_use]
    pub const fn blocked(&self) -> &WaitDirtyStageEvidence {
        &self.blocked
    }

    #[must_use]
    pub const fn mismatch(&self) -> &WaitDirtyStageEvidence {
        &self.mismatch
    }

    #[must_use]
    pub const fn wake(&self) -> &WaitDirtyStageEvidence {
        &self.wake
    }

    #[must_use]
    pub const fn final_state(&self) -> &WaitDirtyStageEvidence {
        &self.final_state
    }
}

pub fn run_wait_channel_dirty_delta_guest(
    config: VmConfig,
) -> Result<WaitChannelDirtyDeltaGuestResult, Error> {
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
    let arm_handler = FlatGuestImage::new(
        TASK_WAKE_ARM_HANDLER,
        TASK_WAKE_ARM_HANDLER,
        &WAIT_WAKE_ARM_HANDLER_BYTES,
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
        .expect("fixed wait-channel dirty-delta layout remains valid");
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
    let dirty_tracker = vm.register_guest_memory_with_dirty_tracking(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(layout.privilege_layout())?;
    let lapic = vcpu.configure_legacy_pic_extint()?;
    let mut port_io = PortIoBus::with_debug_port();

    let setup_dirty = vm.harvest_slot0_dirty(dirty_tracker)?;
    let clean_baseline = vm.harvest_slot0_dirty(dirty_tracker)?;
    require_clean_dirty_bitmap("wait dirty pre-run baseline", &clean_baseline)?;
    let context_page_index = dirty_tracker.page_index(TASK_CONTEXT_PAGE_ADDR)?;

    let blocked_io = run_queue_debug_output(&mut vcpu, &mut port_io, b'K', "wait dirty block A")?;
    let blocked_wait = read_wait_channel_snapshot(
        vm.guest_memory().expect("registered wait dirty memory remains VM-owned"),
    )?;
    require_wait_blocked(blocked_wait)?;
    let blocked = harvest_wait_dirty_stage(&vm, dirty_tracker, "wait dirty blocked transition")?;

    let first_select_io = run_queue_debug_output(
        &mut vcpu,
        &mut port_io,
        b'1',
        "wait dirty queue selects B",
    )?;
    let first_selection = read_queue_snapshot(
        vm.guest_memory().expect("registered wait dirty memory remains VM-owned"),
    )?;
    require_first_queue_selection(first_selection)?;

    let scheduler_a_io = run_queue_debug_output(
        &mut vcpu,
        &mut port_io,
        b'A',
        "wait dirty B scheduler handoff",
    )?;
    let mismatch_io = run_queue_debug_output(
        &mut vcpu,
        &mut port_io,
        b'X',
        "wait dirty wrong wake attempt",
    )?;
    let mismatch_wait = read_wait_channel_snapshot(
        vm.guest_memory().expect("registered wait dirty memory remains VM-owned"),
    )?;
    require_wait_mismatch(mismatch_wait)?;
    let mismatch = harvest_wait_dirty_stage(&vm, dirty_tracker, "wait dirty mismatch transition")?;

    let armed_io = run_queue_debug_output(&mut vcpu, &mut port_io, b'P', "wait dirty wake arm")?;
    let armed = vcpu.registers()?;
    require_queue_interrupt_disabled_flags("wait dirty wake arm state", armed.rflags)?;

    let timer_irq = vm
        .duplicate_irq_line_handle()
        .map_err(|source| queue_vm_error("duplicate wait dirty wake IRQ handle", source))?;
    let watchdog_irq = vm
        .duplicate_irq_line_handle()
        .map_err(|source| queue_vm_error("duplicate wait dirty watchdog handle", source))?;
    timer_irq
        .set_gsi_level(TASK_WAKE_GSI, false)
        .map_err(|source| queue_vm_error("preflight wait dirty IRQ line", source))?;
    watchdog_irq
        .set_gsi_level(TASK_WAKE_GSI, false)
        .map_err(|source| queue_vm_error("preflight wait dirty watchdog line", source))?;

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
            "wait dirty correct external wake",
        )?;
        let wake_wait = read_wait_channel_snapshot(
            vm.guest_memory().expect("registered wait dirty memory remains VM-owned"),
        )?;
        require_wait_woken(wake_wait)?;
        let wake = harvest_wait_dirty_stage(&vm, dirty_tracker, "wait dirty wake transition")?;
        let second_select_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'0',
            "wait dirty queue selects woken A",
        )?;
        let second_selection = read_queue_snapshot(
            vm.guest_memory().expect("registered wait dirty memory remains VM-owned"),
        )?;
        require_second_queue_selection(second_selection)?;
        let scheduler_b_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'B',
            "wait dirty A scheduler handoff",
        )?;
        let restored_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'R',
            "wait dirty restored A",
        )?;
        let done_io = run_queue_debug_output(
            &mut vcpu,
            &mut port_io,
            b'D',
            "wait dirty completion",
        )?;
        let final_state = harvest_wait_dirty_stage(&vm, dirty_tracker, "wait dirty final scheduler state")?;
        Ok((
            wake_io,
            wake_wait,
            wake,
            second_select_io,
            second_selection,
            scheduler_b_io,
            restored_io,
            done_io,
            final_state,
        ))
    })();

    let _ = watchdog_cancel_tx.send(());
    let timer_result = timer_worker.join().map_err(|_| {
        verification_error("join wait dirty timer", "wait dirty timer worker panicked")
    })?;
    let watchdog_fired = watchdog_worker
        .join()
        .map_err(|_| verification_error("join wait dirty watchdog", "wait dirty watchdog panicked"))?
        .map_err(|source| queue_vm_error("wait dirty watchdog GSI", source))?;
    timer_result.map_err(|source| queue_vm_error("wait dirty wake GSI", source))?;
    if watchdog_fired {
        return Err(verification_error(
            "wait dirty watchdog",
            "watchdog injected fallback GSI; correct-channel wake was not independently proven",
        ));
    }

    let (
        wake_io,
        wake_wait,
        wake,
        second_select_io,
        second_selection,
        scheduler_b_io,
        restored_io,
        done_io,
        final_state,
    ) = execution?;
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
            "wait dirty proof",
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
    let guest_memory = vm.guest_memory().expect("registered wait dirty memory remains VM-owned");
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

    let wait = WaitChannelGuestResult {
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
    };

    Ok(WaitChannelDirtyDeltaGuestResult {
        wait,
        context_page_index,
        setup_dirty,
        clean_baseline,
        blocked,
        mismatch,
        wake,
        final_state,
    })
}

fn harvest_wait_dirty_stage(
    vm: &crate::kvm::Vm,
    tracker: crate::kvm::sys::Slot0DirtyTracker,
    stage: &'static str,
) -> Result<WaitDirtyStageEvidence, Error> {
    let dirty = vm.harvest_slot0_dirty(tracker)?;
    if !tracker.bitmap_contains_page(&dirty, TASK_CONTEXT_PAGE_ADDR)? {
        return Err(verification_error(
            stage,
            format!(
                "task-context page {:#x} (page {}) was not dirty in bitmap {dirty:?}",
                TASK_CONTEXT_PAGE_ADDR.get(),
                tracker.page_index(TASK_CONTEXT_PAGE_ADDR)?
            ),
        ));
    }
    let cleared = vm.harvest_slot0_dirty(tracker)?;
    require_clean_dirty_bitmap(stage, &cleared)?;
    Ok(WaitDirtyStageEvidence { dirty, cleared })
}

fn require_clean_dirty_bitmap(stage: &'static str, bitmap: &[u64]) -> Result<(), Error> {
    if bitmap.iter().any(|word| *word != 0) {
        return Err(verification_error(
            stage,
            format!("expected dirty bitmap to be clear without guest re-entry, got {bitmap:?}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod wait_dirty_delta_tests {
    use super::*;

    #[test]
    fn task_context_page_is_page_48_of_the_two_megabyte_slot() {
        assert_eq!(TASK_CONTEXT_PAGE_ADDR.get(), 0x30000);
        assert_eq!(
            TASK_CONTEXT_PAGE_ADDR.get() / crate::memory::KVM_MEMORY_ALIGNMENT,
            48
        );
        assert_eq!(
            LONG_MODE_IDENTITY_MAP_SIZE / crate::memory::KVM_MEMORY_ALIGNMENT,
            512
        );
    }
}
