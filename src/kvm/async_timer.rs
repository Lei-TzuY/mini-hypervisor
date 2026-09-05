const ASYNC_TIMER_READY_BYTE: u8 = b'R';
const ASYNC_TIMER_ARMED_BYTE: u8 = b'A';
const ASYNC_TIMER_HANDLER_BYTE: u8 = b'T';
const ASYNC_TIMER_WOKE_BYTE: u8 = b'W';
const ASYNC_TIMER_DONE_BYTE: u8 = b'D';
const ASYNC_TIMER_DELAY_MILLIS: u64 = 10;
const ASYNC_TIMER_WATCHDOG_SECONDS: u64 = 5;

const ASYNC_TIMER_GUEST_BYTES: [u8; 56] = [
    0xfa, // cli
    0xb0, 0x11, 0xe6, 0x20, 0xe6, 0xa0, // ICW1: initialize master and slave PICs
    0xb0, 0x40, 0xe6, 0x21, // ICW2: master IRQ0..7 -> vectors 0x40..0x47
    0xb0, 0x48, 0xe6, 0xa1, // ICW2: slave IRQ8..15 -> vectors 0x48..0x4f
    0xb0, 0x04, 0xe6, 0x21, // ICW3: master has slave on IRQ2
    0xb0, 0x02, 0xe6, 0xa1, // ICW3: slave cascade identity 2
    0xb0, 0x01, 0xe6, 0x21, 0xe6, 0xa1, // ICW4: 8086 mode on both PICs
    0xb0, 0xfe, 0xe6, 0x21, // OCW1: unmask only master IRQ0
    0xb0, 0xff, 0xe6, 0xa1, // OCW1: mask every slave IRQ
    0xb0, ASYNC_TIMER_READY_BYTE, 0xe6, 0xe9, // readiness while IF remains clear
    0xb0, ASYNC_TIMER_ARMED_BYTE, 0xe6, 0xe9, // explicit host timer-arm barrier
    0xfb, // sti
    0xf4, // hlt -- STI shadow makes an already-pending IRQ wake this HLT safely
    0xb0, ASYNC_TIMER_WOKE_BYTE, 0xe6, 0xe9, // resumed mainline after timer handler
    0xb0, ASYNC_TIMER_DONE_BYTE, 0xe6, 0xe9, // terminal userspace barrier
    0xf4, // safety fallback; host deliberately does not re-enter after D
];

const ASYNC_TIMER_HANDLER_BYTES: [u8; 10] = [
    0xb0, ASYNC_TIMER_HANDLER_BYTE, 0xe6, 0xe9, // timer interrupt-handler proof
    0xb0, 0x20, 0xe6, 0x20, // non-specific EOI to the master PIC
    0x48, 0xcf, // iretq
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncTimerInterruptGuestResult {
    gsi: u32,
    vector: u8,
    lapic_spiv: u32,
    lapic_lint0: u32,
    armed_rflags: u64,
    completion_rflags: u64,
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
}

impl AsyncTimerInterruptGuestResult {
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

impl KvmBackend {
    pub const ASYNC_TIMER_GSI: u32 = Self::IRQCHIP_GSI;
    pub const ASYNC_TIMER_VECTOR: u8 = Self::IRQCHIP_VECTOR;
    pub const ASYNC_TIMER_PROOF: &'static [u8; 5] = b"RATWD";

    pub fn run_async_timer_interrupt_guest(
        config: VmConfig,
    ) -> Result<AsyncTimerInterruptGuestResult, Error> {
        let guest = FlatGuestImage::new(
            LONG_MODE_INTERRUPT_GUEST_ENTRY,
            LONG_MODE_INTERRUPT_GUEST_ENTRY,
            &ASYNC_TIMER_GUEST_BYTES,
        )?;
        let handler = FlatGuestImage::new(
            LONG_MODE_INTERRUPT_HANDLER,
            LONG_MODE_INTERRUPT_HANDLER,
            &ASYNC_TIMER_HANDLER_BYTES,
        )?;

        let backend = Self::open()?;
        let mut vm = backend.create_vm_with_irqchip()?;
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
        let layout = LongModeInterruptLayout::new(
            memory.region(),
            guest.entry(),
            LONG_MODE_INTERRUPT_STACK_POINTER,
            Self::ASYNC_TIMER_VECTOR,
            handler.entry(),
        )
        .expect("fixed deterministic async-timer fixture layout remains valid");
        layout.install_tables(&mut memory)?;
        guest.load(&mut memory)?;
        handler.load(&mut memory)?;
        vm.register_guest_memory(memory)?;

        debug_assert_eq!(config.vcpu_count(), 1);
        let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
        vcpu.initialize_long_mode_interrupts(&layout)?;
        let lapic = vcpu.configure_legacy_pic_extint()?;
        let mut port_io = PortIoBus::with_debug_port();

        let readiness_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            ASYNC_TIMER_READY_BYTE,
            "async timer readiness output",
        )?;
        let armed_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            ASYNC_TIMER_ARMED_BYTE,
            "async timer armed barrier",
        )?;
        let armed = vcpu.registers()?;
        require_interrupt_disabled_flags("async timer armed barrier state", armed.rflags)?;

        let timer_irq = vm
            .duplicate_irq_line_handle()
            .map_err(|source| async_timer_vm_error("duplicate async timer IRQ-line handle", source))?;
        let watchdog_irq = vm
            .duplicate_irq_line_handle()
            .map_err(|source| async_timer_vm_error("duplicate async timer watchdog IRQ-line handle", source))?;

        // Validate both duplicate descriptors before the guest is allowed to enter HLT. This makes
        // descriptor/setup failures synchronous instead of turning them into an indefinitely blocked
        // KVM_RUN. Deasserting an already-low line is architecturally inert.
        timer_irq
            .set_gsi_level(Self::ASYNC_TIMER_GSI, false)
            .map_err(|source| async_timer_vm_error("preflight async timer IRQ-line handle", source))?;
        watchdog_irq
            .set_gsi_level(Self::ASYNC_TIMER_GSI, false)
            .map_err(|source| async_timer_vm_error("preflight async timer watchdog IRQ-line handle", source))?;

        let timer_worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(
                ASYNC_TIMER_DELAY_MILLIS,
            ));
            timer_irq.pulse_gsi_edge(KvmBackend::ASYNC_TIMER_GSI)
        });

        let (watchdog_cancel_tx, watchdog_cancel_rx) = std::sync::mpsc::channel::<()>();
        let watchdog_worker = std::thread::spawn(move || -> io::Result<bool> {
            match watchdog_cancel_rx.recv_timeout(std::time::Duration::from_secs(
                ASYNC_TIMER_WATCHDOG_SECONDS,
            )) {
                Ok(()) => Ok(false),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    watchdog_irq.pulse_gsi_edge(KvmBackend::ASYNC_TIMER_GSI)?;
                    Ok(true)
                }
            }
        });

        // The guest has IF clear at A, then executes the indivisible handoff `sti; hlt` on the next
        // KVM_RUN. x86 STI shadow defers a pending maskable interrupt until after the following HLT
        // instruction, so correctness does not depend on whether the host timer fires just before or
        // just after KVM reaches HLT. Either way, reaching T proves the timer edge crossed the PIC
        // path and woke the HLT handoff before the mainline could emit W.
        let execution = (|| -> Result<_, Error> {
            let handler_io = run_expected_debug_output(
                &mut vcpu,
                &mut port_io,
                ASYNC_TIMER_HANDLER_BYTE,
                "async timer handler output",
            )?;
            let woke_io = run_expected_debug_output(
                &mut vcpu,
                &mut port_io,
                ASYNC_TIMER_WOKE_BYTE,
                "async timer resumed-main output",
            )?;
            let completion_io = run_expected_debug_output(
                &mut vcpu,
                &mut port_io,
                ASYNC_TIMER_DONE_BYTE,
                "async timer completion barrier",
            )?;
            let completion = vcpu.registers()?;
            require_interrupt_enabled_flags("async timer completion state", completion.rflags)?;
            Ok((handler_io, woke_io, completion_io, completion.rflags))
        })();

        // Cancel the watchdog on every non-hanging path, then join both workers before interpreting
        // the guest proof. If the real timer failed or panicked, the watchdog can wake KVM_RUN so CI
        // terminates, but `watchdog_fired` is a hard failure and can never be accepted as timer proof.
        let _ = watchdog_cancel_tx.send(());
        let timer_result = join_async_timer_worker(timer_worker)?;
        let watchdog_fired = join_async_timer_watchdog(watchdog_worker)?;

        if let Err(source) = timer_result {
            return Err(async_timer_vm_error("async timer GSI pulse", source));
        }
        if watchdog_fired {
            return Err(verification_error(
                "async timer watchdog",
                "watchdog injected a fallback GSI; the one-shot timer wakeup was not independently proven",
            ));
        }

        let (handler_io, woke_io, completion_io, completion_rflags) = execution?;
        let io_exits = vec![
            readiness_io,
            armed_io,
            handler_io,
            woke_io,
            completion_io,
        ];
        let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
        if proof.as_slice() != Self::ASYNC_TIMER_PROOF
            || io_exits.len() != Self::ASYNC_TIMER_PROOF.len()
        {
            return Err(verification_error(
                "async timer interrupt execution proof",
                format!(
                    "expected exact proof {:?} across {} byte-wide I/O exits, got proof {:?} across {} exits",
                    Self::ASYNC_TIMER_PROOF,
                    Self::ASYNC_TIMER_PROOF.len(),
                    proof,
                    io_exits.len()
                ),
            ));
        }

        Ok(AsyncTimerInterruptGuestResult {
            gsi: Self::ASYNC_TIMER_GSI,
            vector: Self::ASYNC_TIMER_VECTOR,
            lapic_spiv: lapic.spiv(),
            lapic_lint0: lapic.lint0(),
            armed_rflags: armed.rflags,
            completion_rflags,
            io_exits,
            proof,
        })
    }
}

fn require_interrupt_disabled_flags(operation: &'static str, rflags: u64) -> Result<(), Error> {
    if rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || rflags & X86_RFLAGS_INTERRUPT_ENABLE != 0
    {
        return Err(verification_error(
            operation,
            format!(
                "expected architectural RFLAGS bit 1 set and IF clear, got RFLAGS {rflags:#x}"
            ),
        ));
    }
    Ok(())
}

fn async_timer_vm_error(operation: &'static str, source: io::Error) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VmOperation { operation, source })
}

fn join_async_timer_worker(
    worker: std::thread::JoinHandle<io::Result<()>>,
) -> Result<io::Result<()>, Error> {
    worker.join().map_err(|_| {
        verification_error(
            "join async timer worker",
            "async timer worker panicked before reporting its GSI result",
        )
    })
}

fn join_async_timer_watchdog(
    worker: std::thread::JoinHandle<io::Result<bool>>,
) -> Result<bool, Error> {
    worker
        .join()
        .map_err(|_| {
            verification_error(
                "join async timer watchdog",
                "async timer watchdog panicked before reporting whether it fired",
            )
        })?
        .map_err(|source| async_timer_vm_error("async timer watchdog fallback GSI", source))
}

#[cfg(test)]
mod async_timer_tests {
    use super::*;

    #[test]
    fn deterministic_async_timer_guest_has_cli_arm_then_sti_hlt_handoff() {
        assert_eq!(ASYNC_TIMER_GUEST_BYTES.len(), 56);
        assert_eq!(
            &ASYNC_TIMER_GUEST_BYTES[37..41],
            &[0xb0, b'R', 0xe6, 0xe9]
        );
        assert_eq!(
            &ASYNC_TIMER_GUEST_BYTES[41..45],
            &[0xb0, b'A', 0xe6, 0xe9]
        );
        assert_eq!(&ASYNC_TIMER_GUEST_BYTES[45..47], &[0xfb, 0xf4]);
        assert_eq!(
            &ASYNC_TIMER_GUEST_BYTES[47..51],
            &[0xb0, b'W', 0xe6, 0xe9]
        );
        assert_eq!(
            &ASYNC_TIMER_GUEST_BYTES[51..55],
            &[0xb0, b'D', 0xe6, 0xe9]
        );
        assert_eq!(ASYNC_TIMER_GUEST_BYTES[55], 0xf4);
        assert_eq!(ASYNC_TIMER_HANDLER_BYTES.len(), 10);
        assert_eq!(ASYNC_TIMER_HANDLER_BYTES[1], b'T');
        assert_eq!(KvmBackend::ASYNC_TIMER_GSI, 0);
        assert_eq!(KvmBackend::ASYNC_TIMER_VECTOR, 0x40);
        assert_eq!(KvmBackend::ASYNC_TIMER_PROOF, b"RATWD");
    }

    #[test]
    fn armed_flag_contract_requires_if_clear_until_sti_hlt_handoff() {
        assert!(require_interrupt_disabled_flags("test", 0x002).is_ok());
        assert!(require_interrupt_disabled_flags("test", 0x202).is_err());
        assert!(require_interrupt_disabled_flags("test", 0x000).is_err());
    }
}
