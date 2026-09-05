use super::level_interrupt::{MMIO_LEVEL_INTERRUPT_ACK_VALUE, MMIO_LEVEL_INTERRUPT_COMMAND_VALUE};
use super::long_mode::{
    LongModeMmioBootLayout, LongModeMmioPageMapping, LONG_MODE_MMIO_DEVICE_GPA,
    LONG_MODE_MMIO_STACK_POINTER, LONG_MODE_MMIO_VIRTUAL_PAGE,
};
use super::multi_device::{MULTI_DEVICE_SECOND_GPA, MULTI_DEVICE_SECOND_VIRTUAL_PAGE};
use super::routing::{LegacyPicMmioInterruptRoute, LegacyPicMmioInterruptRoutes};
use super::{
    MmioBus, MmioDeviceEvent, MmioService, LEVEL_INTERRUPT_ACK_OFFSET,
    LEVEL_INTERRUPT_STATUS_OFFSET, LEVEL_INTERRUPT_STATUS_PENDING,
};
use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError, VmExitError};
use crate::interrupt::{
    LongModeInterruptGate, LongModeInterruptLayout, LONG_MODE_INTERRUPT_GUEST_ENTRY,
    LONG_MODE_INTERRUPT_HANDLER, X86_RFLAGS_INTERRUPT_ENABLE,
};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::vcpu::{MmioDirection, MmioExit, PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId};
use std::io;

pub const DUAL_SOURCE_FIRST_GSI: u32 = 0;
pub const DUAL_SOURCE_SECOND_GSI: u32 = 1;
pub const DUAL_SOURCE_FIRST_VECTOR: u8 = 0x40;
pub const DUAL_SOURCE_SECOND_VECTOR: u8 = 0x41;
pub const DUAL_SOURCE_SECOND_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x1_2000);
pub const DUAL_SOURCE_PROOF: &[u8; 11] = b"A0SCMB1TEND";

const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;
const FIRST_ARMED_BYTE: u8 = b'A';
const FIRST_HANDLER_BYTE: u8 = b'0';
const FIRST_STATUS_BYTE: u8 = b'S';
const FIRST_ACK_COMMITTED_BYTE: u8 = b'C';
const FIRST_RESUMED_BYTE: u8 = b'M';
const SECOND_ARMED_BYTE: u8 = b'B';
const SECOND_HANDLER_BYTE: u8 = b'1';
const SECOND_STATUS_BYTE: u8 = b'T';
const SECOND_ACK_COMMITTED_BYTE: u8 = b'E';
const SECOND_RESUMED_BYTE: u8 = b'N';
const DONE_BYTE: u8 = b'D';
const FAILURE_BYTE: u8 = b'F';

// The master PIC is remapped to vectors 0x40..0x47 and IRQ0+IRQ1 are the only unmasked lines.
// Two virtual MMIO device bases remain live in RBX and RCX. Each command is followed by an
// explicit debug-port completion barrier before userspace is allowed to assert that source's GSI.
const DUAL_SOURCE_GUEST_BYTES: [u8; 86] = [
    0xfa, // cli
    0xb0,
    0x11,
    0xe6,
    0x20,
    0xe6,
    0xa0, // ICW1: initialize master and slave PICs
    0xb0,
    0x40,
    0xe6,
    0x21, // ICW2: master IRQ0..7 -> vectors 0x40..0x47
    0xb0,
    0x48,
    0xe6,
    0xa1, // ICW2: slave IRQ8..15 -> vectors 0x48..0x4f
    0xb0,
    0x04,
    0xe6,
    0x21, // ICW3: master has slave on IRQ2
    0xb0,
    0x02,
    0xe6,
    0xa1, // ICW3: slave cascade identity 2
    0xb0,
    0x01,
    0xe6,
    0x21,
    0xe6,
    0xa1, // ICW4: 8086 mode on both PICs
    0xb0,
    0xfc,
    0xe6,
    0x21, // OCW1: unmask master IRQ0 and IRQ1 only
    0xb0,
    0xff,
    0xe6,
    0xa1, // OCW1: mask every slave IRQ
    0xfb, // sti
    0x90, // complete STI interrupt shadow
    0x48,
    0xbb,
    0x00,
    0x00,
    0x50,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00, // movabs $0x500000, %rbx -- first source VA
    0x48,
    0xb9,
    0x00,
    0x10,
    0x50,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00, // movabs $0x501000, %rcx -- second source VA
    0xc6,
    0x03,
    MMIO_LEVEL_INTERRUPT_COMMAND_VALUE, // first COMMAND
    0xb0,
    FIRST_ARMED_BYTE,
    0xe6,
    0xe9, // first command completion barrier
    0xb0,
    FIRST_RESUMED_BYTE,
    0xe6,
    0xe9, // main resumed after first handler
    0xc6,
    0x01,
    MMIO_LEVEL_INTERRUPT_COMMAND_VALUE, // second COMMAND
    0xb0,
    SECOND_ARMED_BYTE,
    0xe6,
    0xe9, // second command completion barrier
    0xb0,
    SECOND_RESUMED_BYTE,
    0xe6,
    0xe9, // main resumed after second handler
    0xb0,
    DONE_BYTE,
    0xe6,
    0xe9, // final userspace synchronization barrier
    0xf4, // safety fallback; host stops at D
];

const FIRST_HANDLER_BYTES: [u8; 34] = [
    0xb0,
    FIRST_HANDLER_BYTE,
    0xe6,
    0xe9, // handler identity
    0x8a,
    0x43,
    0x01, // STATUS from first device through RBX
    0x3c,
    LEVEL_INTERRUPT_STATUS_PENDING,
    0x75,
    0x12, // jne failure output
    0xb0,
    FIRST_STATUS_BYTE,
    0xe6,
    0xe9,
    0xc6,
    0x43,
    0x02,
    MMIO_LEVEL_INTERRUPT_ACK_VALUE, // ACK first device
    0xb0,
    FIRST_ACK_COMMITTED_BYTE,
    0xe6,
    0xe9,
    0xb0,
    0x20,
    0xe6,
    0x20, // master PIC EOI
    0x48,
    0xcf, // iretq
    0xb0,
    FAILURE_BYTE,
    0xe6,
    0xe9,
    0xf4,
];

const SECOND_HANDLER_BYTES: [u8; 34] = [
    0xb0,
    SECOND_HANDLER_BYTE,
    0xe6,
    0xe9, // handler identity
    0x8a,
    0x41,
    0x01, // STATUS from second device through RCX
    0x3c,
    LEVEL_INTERRUPT_STATUS_PENDING,
    0x75,
    0x12, // jne failure output
    0xb0,
    SECOND_STATUS_BYTE,
    0xe6,
    0xe9,
    0xc6,
    0x41,
    0x02,
    MMIO_LEVEL_INTERRUPT_ACK_VALUE, // ACK second device
    0xb0,
    SECOND_ACK_COMMITTED_BYTE,
    0xe6,
    0xe9,
    0xb0,
    0x20,
    0xe6,
    0x20, // master PIC EOI
    0x48,
    0xcf, // iretq
    0xb0,
    FAILURE_BYTE,
    0xe6,
    0xe9,
    0xf4,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DualSourceMmioInterruptGuestResult {
    routes: Vec<LegacyPicMmioInterruptRoute>,
    lapic_spiv: u32,
    lapic_lint0: u32,
    armed_rflags: [u64; 2],
    completion_rflags: u64,
    assert_event_count: u32,
    deassert_event_count: u32,
    mmio_exits: Vec<MmioExit>,
    io_exits: Vec<PortIoExit>,
    first_writes: Vec<u8>,
    second_writes: Vec<u8>,
    proof: Vec<u8>,
}

impl DualSourceMmioInterruptGuestResult {
    #[must_use]
    pub fn routes(&self) -> &[LegacyPicMmioInterruptRoute] {
        &self.routes
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
    pub const fn armed_rflags(&self) -> [u64; 2] {
        self.armed_rflags
    }

    #[must_use]
    pub const fn completion_rflags(&self) -> u64 {
        self.completion_rflags
    }

    #[must_use]
    pub const fn assert_event_count(&self) -> u32 {
        self.assert_event_count
    }

    #[must_use]
    pub const fn deassert_event_count(&self) -> u32 {
        self.deassert_event_count
    }

    #[must_use]
    pub fn mmio_exits(&self) -> &[MmioExit] {
        &self.mmio_exits
    }

    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn first_writes(&self) -> &[u8] {
        &self.first_writes
    }

    #[must_use]
    pub fn second_writes(&self) -> &[u8] {
        &self.second_writes
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
}

struct SourceSpec {
    device_address: u64,
    armed_byte: u8,
    handler_byte: u8,
    status_byte: u8,
    ack_committed_byte: u8,
    resumed_byte: u8,
}

struct SourceEvidence {
    route: LegacyPicMmioInterruptRoute,
    armed_rflags: u64,
    mmio_exits: [MmioExit; 3],
    io_exits: [PortIoExit; 5],
}

pub fn run_dual_source_mmio_interrupt_guest(
    config: VmConfig,
) -> Result<DualSourceMmioInterruptGuestResult, Error> {
    let guest = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_GUEST_ENTRY,
        LONG_MODE_INTERRUPT_GUEST_ENTRY,
        &DUAL_SOURCE_GUEST_BYTES,
    )?;
    let first_handler = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_HANDLER,
        LONG_MODE_INTERRUPT_HANDLER,
        &FIRST_HANDLER_BYTES,
    )?;
    let second_handler = FlatGuestImage::new(
        DUAL_SOURCE_SECOND_HANDLER,
        DUAL_SOURCE_SECOND_HANDLER,
        &SECOND_HANDLER_BYTES,
    )?;

    let routes = LegacyPicMmioInterruptRoutes::new(vec![
        LegacyPicMmioInterruptRoute::new(LONG_MODE_MMIO_DEVICE_GPA, DUAL_SOURCE_FIRST_GSI)
            .expect("fixed first legacy-PIC route remains valid"),
        LegacyPicMmioInterruptRoute::new(MULTI_DEVICE_SECOND_GPA, DUAL_SOURCE_SECOND_GSI)
            .expect("fixed second legacy-PIC route remains valid"),
    ])
    .expect("fixed dual-source legacy-PIC route set remains unambiguous");
    debug_assert_eq!(routes.routes()[0].vector(), DUAL_SOURCE_FIRST_VECTOR);
    debug_assert_eq!(routes.routes()[1].vector(), DUAL_SOURCE_SECOND_VECTOR);

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let mmio_layout = LongModeMmioBootLayout::with_device_mappings(
        memory.region(),
        guest.entry(),
        LONG_MODE_MMIO_STACK_POINTER,
        vec![
            LongModeMmioPageMapping::new(LONG_MODE_MMIO_VIRTUAL_PAGE, LONG_MODE_MMIO_DEVICE_GPA),
            LongModeMmioPageMapping::new(MULTI_DEVICE_SECOND_VIRTUAL_PAGE, MULTI_DEVICE_SECOND_GPA),
        ],
    )
    .expect("fixed dual-source MMIO mappings remain valid");
    let interrupt_layout = LongModeInterruptLayout::with_gates(
        memory.region(),
        guest.entry(),
        LONG_MODE_MMIO_STACK_POINTER,
        vec![
            LongModeInterruptGate::new(routes.routes()[0].vector(), first_handler.entry()),
            LongModeInterruptGate::new(routes.routes()[1].vector(), second_handler.entry()),
        ],
    )
    .expect("fixed dual-source interrupt gates remain valid");
    interrupt_layout.install_tables(&mut memory)?;
    mmio_layout.install_page_tables(&mut memory)?;
    guest.load(&mut memory)?;
    first_handler.load(&mut memory)?;
    second_handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(&interrupt_layout)?;
    let lapic = vcpu.configure_legacy_pic_extint()?;
    let mut port_io = PortIoBus::with_debug_port();
    let mut mmio = MmioBus::empty();
    mmio.register_level_interrupt_byte_device_at(LONG_MODE_MMIO_DEVICE_GPA)
        .expect("fixed first level device registration remains valid");
    mmio.register_level_interrupt_byte_device_at(MULTI_DEVICE_SECOND_GPA)
        .expect("fixed second level device registration remains valid");

    let first = service_source(
        &vm,
        &mut vcpu,
        &mut port_io,
        &mut mmio,
        &routes,
        SourceSpec {
            device_address: LONG_MODE_MMIO_DEVICE_GPA,
            armed_byte: FIRST_ARMED_BYTE,
            handler_byte: FIRST_HANDLER_BYTE,
            status_byte: FIRST_STATUS_BYTE,
            ack_committed_byte: FIRST_ACK_COMMITTED_BYTE,
            resumed_byte: FIRST_RESUMED_BYTE,
        },
    )?;
    let second = service_source(
        &vm,
        &mut vcpu,
        &mut port_io,
        &mut mmio,
        &routes,
        SourceSpec {
            device_address: MULTI_DEVICE_SECOND_GPA,
            armed_byte: SECOND_ARMED_BYTE,
            handler_byte: SECOND_HANDLER_BYTE,
            status_byte: SECOND_STATUS_BYTE,
            ack_committed_byte: SECOND_ACK_COMMITTED_BYTE,
            resumed_byte: SECOND_RESUMED_BYTE,
        },
    )?;

    let done_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        DONE_BYTE,
        "dual-source MMIO interrupt completion barrier",
    )?;
    let completion = vcpu.registers()?;
    require_interrupt_enabled_flags(
        "dual-source MMIO interrupt completion state",
        completion.rflags,
    )?;
    require_no_event(
        &mut mmio,
        "dual-source MMIO interrupt completion event drain",
    )?;

    let first_writes = mmio
        .writes_at(LONG_MODE_MMIO_DEVICE_GPA)
        .unwrap_or(&[])
        .to_vec();
    let second_writes = mmio
        .writes_at(MULTI_DEVICE_SECOND_GPA)
        .unwrap_or(&[])
        .to_vec();
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    let expected_writes = [
        MMIO_LEVEL_INTERRUPT_COMMAND_VALUE,
        MMIO_LEVEL_INTERRUPT_ACK_VALUE,
    ];
    if first_writes.as_slice() != expected_writes
        || second_writes.as_slice() != expected_writes
        || proof.as_slice() != DUAL_SOURCE_PROOF
    {
        return Err(verification_error(
            "dual-source MMIO interrupt execution proof",
            format!(
                "expected both write traces {:?} and proof {:?}; got first {:?}, second {:?}, proof {:?}",
                expected_writes, DUAL_SOURCE_PROOF, first_writes, second_writes, proof
            ),
        ));
    }
    if first.route.gsi() != DUAL_SOURCE_FIRST_GSI
        || first.route.vector() != DUAL_SOURCE_FIRST_VECTOR
        || second.route.gsi() != DUAL_SOURCE_SECOND_GSI
        || second.route.vector() != DUAL_SOURCE_SECOND_VECTOR
    {
        return Err(verification_error(
            "dual-source MMIO interrupt route identity",
            format!(
                "unexpected routes: first {:?}, second {:?}",
                first.route, second.route
            ),
        ));
    }

    let mut mmio_exits = Vec::with_capacity(6);
    mmio_exits.extend(first.mmio_exits);
    mmio_exits.extend(second.mmio_exits);
    let mut io_exits = Vec::with_capacity(DUAL_SOURCE_PROOF.len());
    io_exits.extend(first.io_exits);
    io_exits.extend(second.io_exits);
    io_exits.push(done_io);

    Ok(DualSourceMmioInterruptGuestResult {
        routes: routes.routes().to_vec(),
        lapic_spiv: lapic.spiv(),
        lapic_lint0: lapic.lint0(),
        armed_rflags: [first.armed_rflags, second.armed_rflags],
        completion_rflags: completion.rflags,
        assert_event_count: 2,
        deassert_event_count: 2,
        mmio_exits,
        io_exits,
        first_writes,
        second_writes,
        proof,
    })
}

fn service_source(
    vm: &crate::kvm::Vm,
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    mmio: &mut MmioBus,
    routes: &LegacyPicMmioInterruptRoutes,
    spec: SourceSpec,
) -> Result<SourceEvidence, Error> {
    let command_exit = expect_mmio(vcpu, "dual-source MMIO command")?;
    validate_write(
        &command_exit,
        spec.device_address,
        MMIO_LEVEL_INTERRUPT_COMMAND_VALUE,
        "dual-source MMIO command",
    )?;
    require_write_service(
        mmio.dispatch(&command_exit)?,
        "dual-source MMIO command service",
    )?;

    let armed_io = run_expected_debug_output(
        vcpu,
        port_io,
        spec.armed_byte,
        "dual-source MMIO armed barrier",
    )?;
    let armed = vcpu.registers()?;
    require_interrupt_enabled_flags("dual-source MMIO armed state", armed.rflags)?;

    let route = require_routed_event(
        mmio,
        routes,
        spec.device_address,
        MmioDeviceEvent::InterruptLineAssertRequested,
        "dual-source MMIO assert event",
    )?;
    require_no_event(mmio, "dual-source MMIO duplicate assert event")?;
    vm.set_gsi_level(route.gsi(), true)?;

    let handler_io = run_expected_debug_output(
        vcpu,
        port_io,
        spec.handler_byte,
        "dual-source MMIO handler identity",
    )?;

    let status_exit = expect_mmio(vcpu, "dual-source MMIO status read")?;
    validate_read(
        &status_exit,
        spec.device_address + LEVEL_INTERRUPT_STATUS_OFFSET,
        "dual-source MMIO status read",
    )?;
    let response = match mmio.dispatch(&status_exit)? {
        MmioService::Read(response) if response == [LEVEL_INTERRUPT_STATUS_PENDING] => response,
        MmioService::Read(response) => {
            return Err(verification_error(
                "dual-source MMIO status service",
                format!(
                    "expected pending status byte {}, got {:?}",
                    LEVEL_INTERRUPT_STATUS_PENDING, response
                ),
            ));
        }
        MmioService::Write => {
            return Err(verification_error(
                "dual-source MMIO status service",
                "status read unexpectedly resolved as a write service",
            ));
        }
    };
    vcpu.write_mmio_read_response(&response)?;
    let status_io = run_expected_debug_output(
        vcpu,
        port_io,
        spec.status_byte,
        "dual-source MMIO status completion",
    )?;

    let ack_exit = expect_mmio(vcpu, "dual-source MMIO ACK write")?;
    validate_write(
        &ack_exit,
        spec.device_address + LEVEL_INTERRUPT_ACK_OFFSET,
        MMIO_LEVEL_INTERRUPT_ACK_VALUE,
        "dual-source MMIO ACK write",
    )?;
    require_write_service(mmio.dispatch(&ack_exit)?, "dual-source MMIO ACK service")?;
    let ack_committed_io = run_expected_debug_output(
        vcpu,
        port_io,
        spec.ack_committed_byte,
        "dual-source MMIO ACK completion barrier",
    )?;

    let deassert_route = require_routed_event(
        mmio,
        routes,
        spec.device_address,
        MmioDeviceEvent::InterruptLineDeassertRequested,
        "dual-source MMIO deassert event",
    )?;
    if deassert_route != route {
        return Err(verification_error(
            "dual-source MMIO line ownership",
            format!(
                "assert route {:?} changed before deassert to {:?}",
                route, deassert_route
            ),
        ));
    }
    require_no_event(mmio, "dual-source MMIO duplicate deassert event")?;
    vm.set_gsi_level(route.gsi(), false)?;

    let resumed_io = run_expected_debug_output(
        vcpu,
        port_io,
        spec.resumed_byte,
        "dual-source MMIO resumed main",
    )?;

    Ok(SourceEvidence {
        route,
        armed_rflags: armed.rflags,
        mmio_exits: [command_exit, status_exit, ack_exit],
        io_exits: [
            armed_io,
            handler_io,
            status_io,
            ack_committed_io,
            resumed_io,
        ],
    })
}

fn require_routed_event(
    mmio: &mut MmioBus,
    routes: &LegacyPicMmioInterruptRoutes,
    expected_device: u64,
    expected_event: MmioDeviceEvent,
    stage: &'static str,
) -> Result<LegacyPicMmioInterruptRoute, Error> {
    let Some(record) = mmio.take_device_event_record() else {
        return Err(verification_error(stage, "expected device event, got none"));
    };
    if record.device_address() != expected_device || record.event() != expected_event {
        return Err(verification_error(
            stage,
            format!(
                "expected source {expected_device:#x} event {expected_event:?}, got source {:#x} event {:?}",
                record.device_address(),
                record.event()
            ),
        ));
    }
    routes
        .route_for_device(record.device_address())
        .ok_or_else(|| {
            verification_error(
                stage,
                format!(
                    "MMIO interrupt source {:#x} has no legacy-PIC route",
                    record.device_address()
                ),
            )
        })
}

fn require_no_event(mmio: &mut MmioBus, stage: &'static str) -> Result<(), Error> {
    if let Some(record) = mmio.take_device_event_record() {
        return Err(verification_error(
            stage,
            format!(
                "unexpected source {:#x} event {:?}",
                record.device_address(),
                record.event()
            ),
        ));
    }
    Ok(())
}

fn expect_mmio(vcpu: &mut Vcpu, stage: &'static str) -> Result<MmioExit, Error> {
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Mmio {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage,
            expected_reason: VcpuExit::Mmio.reason(),
            actual_reason: exit.reason(),
        }));
    }
    vcpu.mmio_exit()
}

fn validate_write(
    exit: &MmioExit,
    address: u64,
    value: u8,
    stage: &'static str,
) -> Result<(), Error> {
    if exit.address() != address
        || exit.direction() != MmioDirection::Write
        || exit.length() != 1
        || exit.write_data() != [value]
    {
        return Err(verification_error(
            stage,
            format!(
                "expected byte write {value:#x} at {address:#x}, got address {:#x}, direction {:?}, length {}, data {:?}",
                exit.address(),
                exit.direction(),
                exit.length(),
                exit.write_data()
            ),
        ));
    }
    Ok(())
}

fn validate_read(exit: &MmioExit, address: u64, stage: &'static str) -> Result<(), Error> {
    if exit.address() != address
        || exit.direction() != MmioDirection::Read
        || exit.length() != 1
        || !exit.write_data().is_empty()
    {
        return Err(verification_error(
            stage,
            format!(
                "expected byte read at {address:#x}, got address {:#x}, direction {:?}, length {}, data {:?}",
                exit.address(),
                exit.direction(),
                exit.length(),
                exit.write_data()
            ),
        ));
    }
    Ok(())
}

fn require_write_service(service: MmioService, stage: &'static str) -> Result<(), Error> {
    if service != MmioService::Write {
        return Err(verification_error(
            stage,
            "MMIO write unexpectedly returned a read response",
        ));
    }
    Ok(())
}

fn run_expected_debug_output(
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    expected: u8,
    stage: &'static str,
) -> Result<PortIoExit, Error> {
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Io {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage,
            expected_reason: VcpuExit::Io.reason(),
            actual_reason: exit.reason(),
        }));
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
            format!(
                "expected byte-wide debug-port output {:?}, got direction {:?}, size {}, port {:#x}, count {}, data {:?}",
                char::from(expected),
                io_exit.direction(),
                io_exit.size(),
                io_exit.port(),
                io_exit.count(),
                io_exit.output_data()
            ),
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

fn require_interrupt_enabled_flags(stage: &'static str, rflags: u64) -> Result<(), Error> {
    if rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || rflags & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
    {
        return Err(verification_error(
            stage,
            format!("expected architectural reserved bit and IF in RFLAGS, got {rflags:#x}"),
        ));
    }
    Ok(())
}

fn verification_error(stage: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation: stage,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_dual_source_guest_and_handlers_are_stable() {
        assert_eq!(DUAL_SOURCE_GUEST_BYTES.len(), 86);
        assert_eq!(FIRST_HANDLER_BYTES.len(), 34);
        assert_eq!(SECOND_HANDLER_BYTES.len(), 34);
        assert_eq!(DUAL_SOURCE_PROOF, b"A0SCMB1TEND");
        assert_eq!(DUAL_SOURCE_FIRST_GSI, 0);
        assert_eq!(DUAL_SOURCE_SECOND_GSI, 1);
        assert_eq!(DUAL_SOURCE_FIRST_VECTOR, 0x40);
        assert_eq!(DUAL_SOURCE_SECOND_VECTOR, 0x41);
        assert_eq!(DUAL_SOURCE_SECOND_HANDLER.get(), 0x1_2000);
        assert_eq!(&DUAL_SOURCE_GUEST_BYTES[29..33], &[0xb0, 0xfc, 0xe6, 0x21]);
        assert_eq!(&FIRST_HANDLER_BYTES[4..7], &[0x8a, 0x43, 0x01]);
        assert_eq!(&SECOND_HANDLER_BYTES[4..7], &[0x8a, 0x41, 0x01]);
    }

    #[test]
    fn route_identity_matches_installed_pic_vectors() {
        let first =
            LegacyPicMmioInterruptRoute::new(LONG_MODE_MMIO_DEVICE_GPA, DUAL_SOURCE_FIRST_GSI)
                .unwrap();
        let second =
            LegacyPicMmioInterruptRoute::new(MULTI_DEVICE_SECOND_GPA, DUAL_SOURCE_SECOND_GSI)
                .unwrap();
        assert_eq!(first.vector(), DUAL_SOURCE_FIRST_VECTOR);
        assert_eq!(second.vector(), DUAL_SOURCE_SECOND_VECTOR);
        assert_ne!(first.device_address(), second.device_address());
        assert_ne!(first.gsi(), second.gsi());
        assert_ne!(first.vector(), second.vector());
    }
}
