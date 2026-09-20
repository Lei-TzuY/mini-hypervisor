#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostRegistrationSpecPair {
    specs: [HostRegistrationSpec; 2],
}

impl HostRegistrationSpecPair {
    pub(crate) fn new(
        mut specs: [HostRegistrationSpec; 2],
    ) -> Result<Self, crate::error::Error> {
        specs.sort_unstable_by_key(|spec| spec.doorbell_address());
        let first_end = specs[0]
            .doorbell_address()
            .checked_add(u64::from(specs[0].doorbell_length()))
            .expect("HostRegistrationSpec validation already rejected address overflow");
        if first_end > specs[1].doorbell_address() {
            return Err(host_registration_error(
                "validate host registration pair",
                format!(
                    "host-registration doorbell ranges overlap: {:#x}+{:#x} and {:#x}+{:#x}",
                    specs[0].doorbell_address(),
                    specs[0].doorbell_length(),
                    specs[1].doorbell_address(),
                    specs[1].doorbell_length()
                ),
            ));
        }
        if specs[0].gsi() == specs[1].gsi() {
            return Err(host_registration_error(
                "validate host registration pair",
                format!(
                    "host-registration pair reuses GSI {}",
                    specs[0].gsi()
                ),
            ));
        }
        Ok(Self { specs })
    }

    #[must_use]
    pub(crate) const fn specs(self) -> [HostRegistrationSpec; 2] {
        self.specs
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostRegistrationPairCheckpoint {
    pair: HostRegistrationSpecPair,
}

impl HostRegistrationPairCheckpoint {
    #[must_use]
    pub(crate) const fn capture(pair: HostRegistrationSpecPair) -> Self {
        Self { pair }
    }

    pub(crate) fn reconstruct(
        self,
        backend: &KvmBackend,
        vm: &Vm,
    ) -> Result<ReconstructedHostRegistrationPair, crate::error::Error> {
        ReconstructedHostRegistrationPair::reconstruct(backend, vm, self.pair)
    }
}

#[derive(Debug)]
pub(crate) struct ReconstructedHostRegistrationPair {
    registrations: [ReconstructedHostRegistrations; 2],
}

impl ReconstructedHostRegistrationPair {
    fn reconstruct(
        backend: &KvmBackend,
        vm: &Vm,
        pair: HostRegistrationSpecPair,
    ) -> Result<Self, crate::error::Error> {
        let specs = pair.specs();
        let first = ReconstructedHostRegistrations::reconstruct(backend, vm, specs[0])?;
        let second = match ReconstructedHostRegistrations::reconstruct(backend, vm, specs[1]) {
            Ok(second) => second,
            Err(second_error) => match first.deassign(vm) {
                Ok(()) => return Err(second_error),
                Err(cleanup_error) => {
                    return Err(host_registration_error(
                        "rollback first host registration after second reconstruction failure",
                        format!(
                            "second reconstruction failed: {second_error}; first rollback also failed: {cleanup_error}"
                        ),
                    ))
                }
            },
        };
        Ok(Self {
            registrations: [first, second],
        })
    }

    fn registration(
        &self,
        index: usize,
    ) -> Result<&ReconstructedHostRegistrations, crate::error::Error> {
        self.registrations.get(index).ok_or_else(|| {
            host_registration_error(
                "select reconstructed host registration",
                format!("registration index {index} is outside the fixed pair"),
            )
        })
    }

    pub(crate) fn wait_doorbell(
        &self,
        index: usize,
        timeout_millis: i32,
    ) -> Result<u64, crate::error::Error> {
        self.registration(index)?.wait_doorbell(timeout_millis)
    }

    pub(crate) fn signal_irq(&self, index: usize) -> Result<(), crate::error::Error> {
        self.registration(index)?.signal_irq()
    }

    pub(crate) fn deassign(self, vm: &Vm) -> Result<(), crate::error::Error> {
        let [first, second] = self.registrations;
        let second_result = second.deassign(vm);
        let first_result = first.deassign(vm);
        match (second_result, first_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(second_error), Err(first_error)) => Err(host_registration_error(
                "deassign reconstructed host registration pair",
                format!(
                    "second registration cleanup failed: {second_error}; first registration cleanup failed: {first_error}"
                ),
            )),
        }
    }
}

pub const TWO_HOST_REGISTRATION_FIRST_BAR: u64 =
    crate::mmio::long_mode::LONG_MODE_MMIO_DEVICE_GPA;
pub const TWO_HOST_REGISTRATION_SECOND_BAR: u64 =
    crate::mmio::multi_device::MULTI_DEVICE_SECOND_GPA;
pub const TWO_HOST_REGISTRATION_FIRST_DOORBELL: u64 = TWO_HOST_REGISTRATION_FIRST_BAR + 0x100;
pub const TWO_HOST_REGISTRATION_SECOND_DOORBELL: u64 = TWO_HOST_REGISTRATION_SECOND_BAR + 0x100;
pub const TWO_HOST_REGISTRATION_FIRST_GSI: u32 = 0;
pub const TWO_HOST_REGISTRATION_SECOND_GSI: u32 = 1;
pub const TWO_HOST_REGISTRATION_FIRST_VECTOR: u8 = 0x40;
pub const TWO_HOST_REGISTRATION_SECOND_VECTOR: u8 = 0x41;
pub const TWO_HOST_REGISTRATION_PROOF: &[u8; 15] = b"RA0MB1NCE0PF1QD";

const TWO_HOST_REGISTRATION_SECOND_HANDLER: crate::memory::GuestPhysAddr =
    crate::memory::GuestPhysAddr::new(0x1_2000);
const TWO_HOST_REGISTRATION_WAIT_MILLIS: i32 = 5_000;
const TWO_HOST_REGISTRATION_READY: u8 = b'R';
const TWO_HOST_REGISTRATION_FIRST_ARMED_GEN1: u8 = b'A';
const TWO_HOST_REGISTRATION_FIRST_RESUMED_GEN1: u8 = b'M';
const TWO_HOST_REGISTRATION_SECOND_ARMED_GEN1: u8 = b'B';
const TWO_HOST_REGISTRATION_SECOND_RESUMED_GEN1: u8 = b'N';
const TWO_HOST_REGISTRATION_RECONSTRUCT: u8 = b'C';
const TWO_HOST_REGISTRATION_FIRST_ARMED_GEN2: u8 = b'E';
const TWO_HOST_REGISTRATION_FIRST_RESUMED_GEN2: u8 = b'P';
const TWO_HOST_REGISTRATION_SECOND_ARMED_GEN2: u8 = b'F';
const TWO_HOST_REGISTRATION_SECOND_RESUMED_GEN2: u8 = b'Q';
const TWO_HOST_REGISTRATION_DONE: u8 = b'D';
const TWO_HOST_REGISTRATION_FIRST_HANDLER: u8 = b'0';
const TWO_HOST_REGISTRATION_SECOND_HANDLER_BYTE: u8 = b'1';

const TWO_HOST_REGISTRATION_HANDLER0_BYTES: [u8; 10] = [
    0xb0,
    TWO_HOST_REGISTRATION_FIRST_HANDLER,
    0xe6,
    0xe9,
    0xb0,
    0x20,
    0xe6,
    0x20,
    0x48,
    0xcf,
];

const TWO_HOST_REGISTRATION_HANDLER1_BYTES: [u8; 10] = [
    0xb0,
    TWO_HOST_REGISTRATION_SECOND_HANDLER_BYTE,
    0xe6,
    0xe9,
    0xb0,
    0x20,
    0xe6,
    0x20,
    0x48,
    0xcf,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoHostRegistrationAccelerationResult {
    doorbells: [u64; 2],
    gsis: [u32; 2],
    vectors: [u8; 2],
    generation_doorbell_events: [[u64; 2]; 2],
    proof: Vec<u8>,
    completion_rflags: u64,
}

impl TwoHostRegistrationAccelerationResult {
    #[must_use]
    pub const fn doorbells(&self) -> [u64; 2] {
        self.doorbells
    }

    #[must_use]
    pub const fn gsis(&self) -> [u32; 2] {
        self.gsis
    }

    #[must_use]
    pub const fn vectors(&self) -> [u8; 2] {
        self.vectors
    }

    #[must_use]
    pub const fn generation_doorbell_events(&self) -> [[u64; 2]; 2] {
        self.generation_doorbell_events
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    #[must_use]
    pub const fn completion_rflags(&self) -> u64 {
        self.completion_rflags
    }
}

impl KvmBackend {
    pub fn run_two_host_registration_acceleration_guest(
        config: crate::config::VmConfig,
    ) -> Result<TwoHostRegistrationAccelerationResult, crate::error::Error> {
        run_two_host_registration_acceleration_guest(config)
    }
}

fn run_two_host_registration_acceleration_guest(
    config: crate::config::VmConfig,
) -> Result<TwoHostRegistrationAccelerationResult, crate::error::Error> {
    let guest_bytes = build_two_host_registration_guest();
    let guest = crate::loader::FlatGuestImage::new(
        crate::interrupt::LONG_MODE_INTERRUPT_GUEST_ENTRY,
        crate::interrupt::LONG_MODE_INTERRUPT_GUEST_ENTRY,
        &guest_bytes,
    )?;
    let first_handler = crate::loader::FlatGuestImage::new(
        crate::interrupt::LONG_MODE_INTERRUPT_HANDLER,
        crate::interrupt::LONG_MODE_INTERRUPT_HANDLER,
        &TWO_HOST_REGISTRATION_HANDLER0_BYTES,
    )?;
    let second_handler = crate::loader::FlatGuestImage::new(
        TWO_HOST_REGISTRATION_SECOND_HANDLER,
        TWO_HOST_REGISTRATION_SECOND_HANDLER,
        &TWO_HOST_REGISTRATION_HANDLER1_BYTES,
    )?;

    let routes = crate::mmio::routing::LegacyPicMmioInterruptRoutes::new(vec![
        crate::mmio::routing::LegacyPicMmioInterruptRoute::new(
            TWO_HOST_REGISTRATION_FIRST_BAR,
            TWO_HOST_REGISTRATION_FIRST_GSI,
        )
        .expect("fixed first accelerated route remains valid"),
        crate::mmio::routing::LegacyPicMmioInterruptRoute::new(
            TWO_HOST_REGISTRATION_SECOND_BAR,
            TWO_HOST_REGISTRATION_SECOND_GSI,
        )
        .expect("fixed second accelerated route remains valid"),
    ])
    .expect("fixed two-registration route set remains unambiguous");
    if routes.routes()[0].vector() != TWO_HOST_REGISTRATION_FIRST_VECTOR
        || routes.routes()[1].vector() != TWO_HOST_REGISTRATION_SECOND_VECTOR
    {
        return Err(verification_error(
            "two host-registration route derivation",
            format!("unexpected route set: {:?}", routes.routes()),
        ));
    }

    let first_spec = HostRegistrationSpec::new(
        TWO_HOST_REGISTRATION_FIRST_DOORBELL,
        2,
        0,
        TWO_HOST_REGISTRATION_FIRST_GSI,
    )?;
    let second_spec = HostRegistrationSpec::new(
        TWO_HOST_REGISTRATION_SECOND_DOORBELL,
        2,
        0,
        TWO_HOST_REGISTRATION_SECOND_GSI,
    )?;
    let pair = HostRegistrationSpecPair::new([second_spec, first_spec])?;
    let pair_checkpoint = HostRegistrationPairCheckpoint::capture(pair);
    let canonical_specs = pair.specs();
    if canonical_specs != [first_spec, second_spec] {
        return Err(verification_error(
            "two host-registration canonical ownership",
            format!("unexpected canonical registration pair: {canonical_specs:?}"),
        ));
    }

    let backend = KvmBackend::open()?;
    require_irqfd_capability(&backend)?;
    require_ioeventfd_capability(&backend)?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = crate::memory::GuestMemory::new(
        crate::memory::GuestPhysAddr::new(0),
        crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE,
    )?;
    let mmio_layout = crate::mmio::long_mode::LongModeMmioBootLayout::with_device_mappings(
        memory.region(),
        guest.entry(),
        crate::mmio::long_mode::LONG_MODE_MMIO_STACK_POINTER,
        vec![
            crate::mmio::long_mode::LongModeMmioPageMapping::new(
                crate::mmio::long_mode::LONG_MODE_MMIO_VIRTUAL_PAGE,
                TWO_HOST_REGISTRATION_FIRST_BAR,
            ),
            crate::mmio::long_mode::LongModeMmioPageMapping::new(
                crate::mmio::multi_device::MULTI_DEVICE_SECOND_VIRTUAL_PAGE,
                TWO_HOST_REGISTRATION_SECOND_BAR,
            ),
        ],
    )
    .expect("fixed two-registration MMIO mappings remain valid");
    let interrupt_layout = crate::interrupt::LongModeInterruptLayout::with_gates(
        memory.region(),
        guest.entry(),
        crate::mmio::long_mode::LONG_MODE_MMIO_STACK_POINTER,
        vec![
            crate::interrupt::LongModeInterruptGate::new(
                TWO_HOST_REGISTRATION_FIRST_VECTOR,
                first_handler.entry(),
            ),
            crate::interrupt::LongModeInterruptGate::new(
                TWO_HOST_REGISTRATION_SECOND_VECTOR,
                second_handler.entry(),
            ),
        ],
    )
    .expect("fixed two-registration interrupt gates remain valid");
    interrupt_layout.install_tables(&mut memory)?;
    mmio_layout.install_page_tables(&mut memory)?;
    guest.load(&mut memory)?;
    first_handler.load(&mut memory)?;
    second_handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(crate::vcpu::VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(&interrupt_layout)?;
    let _lapic = vcpu.configure_legacy_pic_extint()?;
    let mut port_io = crate::portio::PortIoBus::with_debug_port();

    let _ready = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        TWO_HOST_REGISTRATION_READY,
        "two host-registration readiness",
    )?;
    let ready = vcpu.registers()?;
    require_interrupt_disabled_flags("two host-registration readiness state", ready.rflags)?;

    let first_generation = pair_checkpoint.reconstruct(&backend, &vm)?;
    let first_events = run_registration_generation(
        &vm,
        &mut vcpu,
        &mut port_io,
        &first_generation,
        TWO_HOST_REGISTRATION_FIRST_ARMED_GEN1,
        TWO_HOST_REGISTRATION_FIRST_RESUMED_GEN1,
        TWO_HOST_REGISTRATION_SECOND_ARMED_GEN1,
        TWO_HOST_REGISTRATION_SECOND_RESUMED_GEN1,
        "first reconstructed generation",
    )?;

    let _reconstruct = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        TWO_HOST_REGISTRATION_RECONSTRUCT,
        "two host-registration reconstruction barrier",
    )?;
    let reconstruct_state = vcpu.registers()?;
    require_interrupt_disabled_flags(
        "two host-registration reconstruction state",
        reconstruct_state.rflags,
    )?;
    first_generation.deassign(&vm)?;

    let second_generation = pair_checkpoint.reconstruct(&backend, &vm)?;
    let second_events = run_registration_generation(
        &vm,
        &mut vcpu,
        &mut port_io,
        &second_generation,
        TWO_HOST_REGISTRATION_FIRST_ARMED_GEN2,
        TWO_HOST_REGISTRATION_FIRST_RESUMED_GEN2,
        TWO_HOST_REGISTRATION_SECOND_ARMED_GEN2,
        TWO_HOST_REGISTRATION_SECOND_RESUMED_GEN2,
        "second reconstructed generation",
    )?;

    let _done = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        TWO_HOST_REGISTRATION_DONE,
        "two host-registration completion barrier",
    )?;
    let completion = vcpu.registers()?;
    require_interrupt_enabled_flags(
        "two host-registration completion state",
        completion.rflags,
    )?;
    second_generation.deassign(&vm)?;

    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != TWO_HOST_REGISTRATION_PROOF {
        return Err(verification_error(
            "two host-registration accelerated proof",
            format!(
                "expected proof {:?}, got {proof:?}",
                TWO_HOST_REGISTRATION_PROOF
            ),
        ));
    }
    if first_events != [1, 1] || second_events != [1, 1] {
        return Err(verification_error(
            "two host-registration doorbell counts",
            format!(
                "expected one event per device per generation, got {first_events:?} and {second_events:?}"
            ),
        ));
    }

    Ok(TwoHostRegistrationAccelerationResult {
        doorbells: [
            TWO_HOST_REGISTRATION_FIRST_DOORBELL,
            TWO_HOST_REGISTRATION_SECOND_DOORBELL,
        ],
        gsis: [
            TWO_HOST_REGISTRATION_FIRST_GSI,
            TWO_HOST_REGISTRATION_SECOND_GSI,
        ],
        vectors: [
            TWO_HOST_REGISTRATION_FIRST_VECTOR,
            TWO_HOST_REGISTRATION_SECOND_VECTOR,
        ],
        generation_doorbell_events: [first_events, second_events],
        proof,
        completion_rflags: completion.rflags,
    })
}

fn run_registration_generation(
    vm: &Vm,
    vcpu: &mut crate::vcpu::Vcpu,
    port_io: &mut crate::portio::PortIoBus,
    registrations: &ReconstructedHostRegistrationPair,
    first_armed: u8,
    first_resumed: u8,
    second_armed: u8,
    second_resumed: u8,
    generation: &'static str,
) -> Result<[u64; 2], crate::error::Error> {
    let _first_armed = run_expected_debug_output(
        vcpu,
        port_io,
        first_armed,
        "two host-registration first doorbell barrier",
    )?;
    let first_state = vcpu.registers()?;
    require_interrupt_disabled_flags(
        "two host-registration first doorbell state",
        first_state.rflags,
    )?;
    let first_count =
        registrations.wait_doorbell(0, TWO_HOST_REGISTRATION_WAIT_MILLIS)?;
    if first_count != 1 {
        return Err(verification_error(
            "two host-registration first doorbell count",
            format!("{generation}: expected 1, got {first_count}"),
        ));
    }
    registrations.signal_irq(0)?;
    run_two_host_registration_irq_handoff(
        vm,
        vcpu,
        port_io,
        TWO_HOST_REGISTRATION_FIRST_HANDLER,
        TWO_HOST_REGISTRATION_FIRST_GSI,
        "two host-registration first IRQ handler",
    )?;
    let _first_resumed = run_expected_debug_output(
        vcpu,
        port_io,
        first_resumed,
        "two host-registration first resumed main",
    )?;

    let _second_armed = run_expected_debug_output(
        vcpu,
        port_io,
        second_armed,
        "two host-registration second doorbell barrier",
    )?;
    let second_state = vcpu.registers()?;
    require_interrupt_disabled_flags(
        "two host-registration second doorbell state",
        second_state.rflags,
    )?;
    let second_count =
        registrations.wait_doorbell(1, TWO_HOST_REGISTRATION_WAIT_MILLIS)?;
    if second_count != 1 {
        return Err(verification_error(
            "two host-registration second doorbell count",
            format!("{generation}: expected 1, got {second_count}"),
        ));
    }
    registrations.signal_irq(1)?;
    run_two_host_registration_irq_handoff(
        vm,
        vcpu,
        port_io,
        TWO_HOST_REGISTRATION_SECOND_HANDLER_BYTE,
        TWO_HOST_REGISTRATION_SECOND_GSI,
        "two host-registration second IRQ handler",
    )?;
    let _second_resumed = run_expected_debug_output(
        vcpu,
        port_io,
        second_resumed,
        "two host-registration second resumed main",
    )?;

    Ok([first_count, second_count])
}

fn run_two_host_registration_irq_handoff(
    vm: &Vm,
    vcpu: &mut crate::vcpu::Vcpu,
    port_io: &mut crate::portio::PortIoBus,
    expected_handler: u8,
    gsi: u32,
    stage: &'static str,
) -> Result<(), crate::error::Error> {
    let watchdog_irq = vm.duplicate_irq_line_handle().map_err(|source| {
        host_registration_error_with_source(
            "duplicate two-registration watchdog IRQ-line handle",
            source,
        )
    })?;
    watchdog_irq.set_gsi_level(gsi, false).map_err(|source| {
        host_registration_error_with_source(
            "preflight two-registration watchdog IRQ-line handle",
            source,
        )
    })?;
    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || -> std::io::Result<bool> {
        match cancel_rx.recv_timeout(std::time::Duration::from_secs(
            ASYNC_TIMER_WATCHDOG_SECONDS,
        )) {
            Ok(()) => Ok(false),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                watchdog_irq.pulse_gsi_edge(gsi)?;
                Ok(true)
            }
        }
    });

    let handler = run_expected_debug_output(vcpu, port_io, expected_handler, stage);
    let _ = cancel_tx.send(());
    let watchdog_fired = join_async_timer_watchdog(watchdog)?;
    if watchdog_fired {
        return Err(verification_error(
            "two host-registration acceleration watchdog",
            format!(
                "{stage}: fallback GSI {gsi} fired; irqfd acceleration was not independently proven"
            ),
        ));
    }
    handler?;
    Ok(())
}

fn build_two_host_registration_guest() -> Vec<u8> {
    let mut code = vec![
        0xfa,
        0xb0, 0x11, 0xe6, 0x20, 0xe6, 0xa0,
        0xb0, 0x40, 0xe6, 0x21,
        0xb0, 0x48, 0xe6, 0xa1,
        0xb0, 0x04, 0xe6, 0x21,
        0xb0, 0x02, 0xe6, 0xa1,
        0xb0, 0x01, 0xe6, 0x21, 0xe6, 0xa1,
        0xb0, 0xfc, 0xe6, 0x21,
        0xb0, 0xff, 0xe6, 0xa1,
    ];
    emit_two_host_registration_debug(&mut code, TWO_HOST_REGISTRATION_READY);
    code.extend_from_slice(&[0x48, 0xbb]);
    code.extend_from_slice(&crate::mmio::long_mode::LONG_MODE_MMIO_VIRTUAL_PAGE.to_le_bytes());
    code.extend_from_slice(&[0x48, 0xb9]);
    code.extend_from_slice(
        &crate::mmio::multi_device::MULTI_DEVICE_SECOND_VIRTUAL_PAGE.to_le_bytes(),
    );

    emit_two_host_registration_doorbell(&mut code, 0x83);
    emit_two_host_registration_debug(&mut code, TWO_HOST_REGISTRATION_FIRST_ARMED_GEN1);
    code.extend_from_slice(&[0xfb, 0xf4]);
    emit_two_host_registration_debug(&mut code, TWO_HOST_REGISTRATION_FIRST_RESUMED_GEN1);

    code.push(0xfa);
    emit_two_host_registration_doorbell(&mut code, 0x81);
    emit_two_host_registration_debug(&mut code, TWO_HOST_REGISTRATION_SECOND_ARMED_GEN1);
    code.extend_from_slice(&[0xfb, 0xf4]);
    emit_two_host_registration_debug(&mut code, TWO_HOST_REGISTRATION_SECOND_RESUMED_GEN1);

    code.push(0xfa);
    emit_two_host_registration_debug(&mut code, TWO_HOST_REGISTRATION_RECONSTRUCT);

    emit_two_host_registration_doorbell(&mut code, 0x83);
    emit_two_host_registration_debug(&mut code, TWO_HOST_REGISTRATION_FIRST_ARMED_GEN2);
    code.extend_from_slice(&[0xfb, 0xf4]);
    emit_two_host_registration_debug(&mut code, TWO_HOST_REGISTRATION_FIRST_RESUMED_GEN2);

    code.push(0xfa);
    emit_two_host_registration_doorbell(&mut code, 0x81);
    emit_two_host_registration_debug(&mut code, TWO_HOST_REGISTRATION_SECOND_ARMED_GEN2);
    code.extend_from_slice(&[0xfb, 0xf4]);
    emit_two_host_registration_debug(&mut code, TWO_HOST_REGISTRATION_SECOND_RESUMED_GEN2);

    emit_two_host_registration_debug(&mut code, TWO_HOST_REGISTRATION_DONE);
    code.push(0xf4);
    code
}

fn emit_two_host_registration_debug(code: &mut Vec<u8>, byte: u8) {
    code.extend_from_slice(&[0xb0, byte, 0xe6, 0xe9]);
}

fn emit_two_host_registration_doorbell(code: &mut Vec<u8>, modrm: u8) {
    code.extend_from_slice(&[0x66, 0xc7, modrm, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);
}

#[cfg(test)]
mod two_host_registration_pair_tests {
    use super::*;

    fn spec(address: u64, length: u32, gsi: u32) -> HostRegistrationSpec {
        HostRegistrationSpec::new(address, length, 0, gsi).unwrap()
    }

    #[test]
    fn pair_is_canonical_non_overlapping_and_gsi_unique() {
        let first = spec(0x1000_0100, 2, 0);
        let second = spec(0x1000_1100, 2, 1);
        let pair = HostRegistrationSpecPair::new([second, first]).unwrap();
        assert_eq!(pair.specs(), [first, second]);

        assert!(HostRegistrationSpecPair::new([
            spec(0x1000, 8, 0),
            spec(0x1004, 4, 1),
        ])
        .is_err());
        assert!(HostRegistrationSpecPair::new([
            spec(0x1000, 2, 0),
            spec(0x2000, 2, 0),
        ])
        .is_err());
    }

    #[test]
    fn pair_checkpoint_contains_only_two_fd_free_semantic_specs() {
        let pair = HostRegistrationSpecPair::new([
            spec(TWO_HOST_REGISTRATION_FIRST_DOORBELL, 2, 0),
            spec(TWO_HOST_REGISTRATION_SECOND_DOORBELL, 2, 1),
        ])
        .unwrap();
        let checkpoint = HostRegistrationPairCheckpoint::capture(pair);
        assert_eq!(checkpoint.pair, pair);
        assert_eq!(
            std::mem::size_of::<HostRegistrationPairCheckpoint>(),
            2 * std::mem::size_of::<HostRegistrationSpec>()
        );
    }

    #[test]
    fn deterministic_two_generation_guest_contract_is_stable() {
        let guest = build_two_host_registration_guest();
        assert!(!guest.is_empty());
        assert_eq!(TWO_HOST_REGISTRATION_PROOF, b"RA0MB1NCE0PF1QD");
        assert_eq!(
            TWO_HOST_REGISTRATION_SECOND_BAR - TWO_HOST_REGISTRATION_FIRST_BAR,
            0x1000
        );
        assert_eq!(
            [
                TWO_HOST_REGISTRATION_FIRST_VECTOR,
                TWO_HOST_REGISTRATION_SECOND_VECTOR,
            ],
            [0x40, 0x41]
        );
    }
}
