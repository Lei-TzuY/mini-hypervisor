use super::{
    BoundedCheckpointPage, BoundedCheckpointPageComparison, BoundedPageSetCheckpointComparison,
    BoundedVcpuPageSetCheckpoint, VcpuStateSnapshot, VcpuStateSnapshotComparison,
};
use crate::error::{Error, HostEnvironmentError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::msr::GuestMsrAccessPolicy;
use crate::kvm::sys::{
    IoapicStateSnapshot, KvmLapicState, MasterPicStateSnapshot, SlavePicStateSnapshot,
};
use crate::kvm::{KvmBackend, Vm};
use crate::loader::FlatGuestImage;
use crate::long_mode::{LongModeBootLayout, LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;

pub const TWO_VCPU_CHECKPOINT_FIRST_ID: VcpuId = VcpuId::BOOT;
pub const TWO_VCPU_CHECKPOINT_SECOND_ID: VcpuId = VcpuId::new(1);
pub const TWO_VCPU_CHECKPOINT_FIRST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x10000);
pub const TWO_VCPU_CHECKPOINT_SECOND_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x11000);
pub const TWO_VCPU_CHECKPOINT_SHARED_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x30000);
pub const TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x1fd000);
pub const TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x1fc000);
pub const TWO_VCPU_CHECKPOINT_FIRST_STACK: u64 = 0x1fdff8;
pub const TWO_VCPU_CHECKPOINT_SECOND_STACK: u64 = 0x1fcff8;
pub const TWO_VCPU_CHECKPOINT_SHARED_MARKER: u8 = b'S';
pub const TWO_VCPU_CHECKPOINT_FIRST_MARKER: u8 = b'0';
pub const TWO_VCPU_CHECKPOINT_SECOND_MARKER: u8 = b'1';
pub const TWO_VCPU_CHECKPOINT_FIRST_PROOF: &[u8; 1] = b"0";
pub const TWO_VCPU_CHECKPOINT_SECOND_PROOF: &[u8; 1] = b"1";
pub const TWO_VCPU_CHECKPOINT_FIRST_CAPTURE_RIP: u64 = 0x1000b;
pub const TWO_VCPU_CHECKPOINT_SECOND_CAPTURE_RIP: u64 = 0x11003;
pub const TWO_VCPU_CHECKPOINT_FIRST_TERMINAL_RIP: u64 = 0x10020;
pub const TWO_VCPU_CHECKPOINT_SECOND_TERMINAL_RIP: u64 = 0x11018;
pub const TWO_VCPU_CHECKPOINT_OWNERSHIP_SET: [GuestPhysAddr; 3] = [
    TWO_VCPU_CHECKPOINT_SHARED_PAGE,
    TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE,
    TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE,
];

const TWO_VCPU_FULL_CONTROLLER_IOAPIC_PIN: usize = 16;
const TWO_VCPU_FULL_CONTROLLER_APIC_SPIV_OFFSET: usize = 0x0f0;
const TWO_VCPU_FULL_CONTROLLER_APIC_LVT0_OFFSET: usize = 0x350;
const TWO_VCPU_FULL_CONTROLLER_APIC_SOFTWARE_ENABLE: u32 = 1 << 8;
const TWO_VCPU_FULL_CONTROLLER_APIC_LVT_MASKED: u32 = 1 << 16;

const TWO_VCPU_FULL_CONTROLLER_FIRST_CAPTURE_MARKER: u8 = b'A';
const TWO_VCPU_FULL_CONTROLLER_SECOND_CAPTURE_MARKER: u8 = b'B';
const TWO_VCPU_FULL_CONTROLLER_MP_STATE_RUNNABLE: u32 = 0;
const TWO_VCPU_FULL_CONTROLLER_MP_STATE_UNINITIALIZED: u32 = 1;
const TWO_VCPU_FULL_CONTROLLER_MP_STATE_HALTED: u32 = 3;
const TWO_VCPU_FULL_CONTROLLER_FIRST_CAPTURE_RIP: u64 = TWO_VCPU_CHECKPOINT_FIRST_ENTRY.get() + 15;
const TWO_VCPU_FULL_CONTROLLER_SECOND_CAPTURE_RIP: u64 = TWO_VCPU_CHECKPOINT_SECOND_ENTRY.get() + 7;
const TWO_VCPU_FULL_CONTROLLER_FIRST_COMPLETION_RIP: u64 =
    TWO_VCPU_CHECKPOINT_FIRST_ENTRY.get() + 36;
const TWO_VCPU_FULL_CONTROLLER_SECOND_COMPLETION_RIP: u64 =
    TWO_VCPU_CHECKPOINT_SECOND_ENTRY.get() + 28;

#[rustfmt::skip]
const TWO_VCPU_FULL_CONTROLLER_FIRST_GUEST_BYTES: [u8; 42] = [
    0xc6, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00, TWO_VCPU_CHECKPOINT_SHARED_MARKER,
    0x6a, TWO_VCPU_CHECKPOINT_FIRST_MARKER,
    0xb0, TWO_VCPU_FULL_CONTROLLER_FIRST_CAPTURE_MARKER, 0xe6, 0xe9, 0x90,
    0x58, 0x3c, TWO_VCPU_CHECKPOINT_FIRST_MARKER, 0x75, 0x11,
    0x8a, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00,
    0x3c, TWO_VCPU_CHECKPOINT_SHARED_MARKER, 0x75, 0x06,
    0xb0, TWO_VCPU_CHECKPOINT_FIRST_MARKER, 0xe6, 0xe9, 0x90,
    0xf4,
    0xb0, b'F', 0xe6, 0xe9, 0xf4,
];

#[rustfmt::skip]
const TWO_VCPU_FULL_CONTROLLER_SECOND_GUEST_BYTES: [u8; 34] = [
    0x6a, TWO_VCPU_CHECKPOINT_SECOND_MARKER,
    0xb0, TWO_VCPU_FULL_CONTROLLER_SECOND_CAPTURE_MARKER, 0xe6, 0xe9, 0x90,
    0x58, 0x3c, TWO_VCPU_CHECKPOINT_SECOND_MARKER, 0x75, 0x11,
    0x8a, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00,
    0x3c, TWO_VCPU_CHECKPOINT_SHARED_MARKER, 0x75, 0x06,
    0xb0, TWO_VCPU_CHECKPOINT_SECOND_MARKER, 0xe6, 0xe9, 0x90,
    0xf4,
    0xb0, b'F', 0xe6, 0xe9, 0xf4,
];

const FIRST_GUEST_BYTES: [u8; 37] = [
    0xc6,
    0x04,
    0x25,
    0x00,
    0x00,
    0x03,
    0x00,
    TWO_VCPU_CHECKPOINT_SHARED_MARKER,
    0x6a,
    TWO_VCPU_CHECKPOINT_FIRST_MARKER,
    0xf4,
    0x58,
    0x3c,
    TWO_VCPU_CHECKPOINT_FIRST_MARKER,
    0x75,
    0x10,
    0x8a,
    0x04,
    0x25,
    0x00,
    0x00,
    0x03,
    0x00,
    0x3c,
    TWO_VCPU_CHECKPOINT_SHARED_MARKER,
    0x75,
    0x05,
    0xb0,
    TWO_VCPU_CHECKPOINT_FIRST_MARKER,
    0xe6,
    0xe9,
    0xf4,
    0xb0,
    b'F',
    0xe6,
    0xe9,
    0xf4,
];

const SECOND_GUEST_BYTES: [u8; 29] = [
    0x6a,
    TWO_VCPU_CHECKPOINT_SECOND_MARKER,
    0xf4,
    0x58,
    0x3c,
    TWO_VCPU_CHECKPOINT_SECOND_MARKER,
    0x75,
    0x10,
    0x8a,
    0x04,
    0x25,
    0x00,
    0x00,
    0x03,
    0x00,
    0x3c,
    TWO_VCPU_CHECKPOINT_SHARED_MARKER,
    0x75,
    0x05,
    0xb0,
    TWO_VCPU_CHECKPOINT_SECOND_MARKER,
    0xe6,
    0xe9,
    0xf4,
    0xb0,
    b'F',
    0xe6,
    0xe9,
    0xf4,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedTwoVcpuCheckpoint {
    primary_id: VcpuId,
    primary: BoundedVcpuPageSetCheckpoint,
    secondary_id: VcpuId,
    secondary: VcpuStateSnapshot,
}

impl BoundedTwoVcpuCheckpoint {
    pub fn capture(
        first: &Vcpu,
        second: &Vcpu,
        msr_policy: &GuestMsrAccessPolicy,
        memory: &GuestMemory,
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let (primary_vcpu, secondary_vcpu) = canonical_vcpu_pair(first, second)?;
        let primary_id = primary_vcpu.id();
        let secondary_id = secondary_vcpu.id();
        let primary = BoundedVcpuPageSetCheckpoint::capture(
            primary_vcpu,
            msr_policy,
            memory,
            page_addresses,
        )?;
        let secondary = secondary_vcpu.capture_state_snapshot(msr_policy)?;
        Ok(Self {
            primary_id,
            primary,
            secondary_id,
            secondary,
        })
    }

    #[must_use]
    pub const fn vcpu_ids(&self) -> [VcpuId; 2] {
        [self.primary_id, self.secondary_id]
    }

    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPage] {
        self.primary.pages()
    }

    pub fn verify(
        &self,
        first: &Vcpu,
        second: &Vcpu,
        memory: &GuestMemory,
    ) -> Result<BoundedTwoVcpuCheckpointComparison, Error> {
        let (primary, secondary) = self.bind_vcpus(first, second)?;
        Ok(BoundedTwoVcpuCheckpointComparison {
            primary_id: self.primary_id,
            primary: self.primary.verify(primary, memory)?,
            secondary_id: self.secondary_id,
            secondary: secondary.verify_state_snapshot(&self.secondary)?,
        })
    }

    pub fn restore_and_verify(
        &self,
        first: &Vcpu,
        second: &Vcpu,
        memory: &mut GuestMemory,
    ) -> Result<BoundedTwoVcpuCheckpointComparison, Error> {
        let (primary, secondary) = self.bind_vcpus(first, second)?;
        Ok(BoundedTwoVcpuCheckpointComparison {
            primary_id: self.primary_id,
            primary: self.primary.restore_and_verify(primary, memory)?,
            secondary_id: self.secondary_id,
            secondary: secondary.restore_and_verify_state_snapshot(&self.secondary)?,
        })
    }

    fn bind_vcpus<'a>(
        &self,
        first: &'a Vcpu,
        second: &'a Vcpu,
    ) -> Result<(&'a Vcpu, &'a Vcpu), Error> {
        let (primary, secondary) = canonical_vcpu_pair(first, second)?;
        if [primary.id(), secondary.id()] != self.vcpu_ids() {
            return Err(two_vcpu_checkpoint_error(
                primary.id(),
                "two-vCPU checkpoint binding",
                format!(
                    "checkpoint owns {:?}, supplied {:?}",
                    self.vcpu_ids(),
                    [primary.id(), secondary.id()]
                ),
            ));
        }
        Ok((primary, secondary))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedTwoVcpuCheckpointComparison {
    primary_id: VcpuId,
    primary: BoundedPageSetCheckpointComparison,
    secondary_id: VcpuId,
    secondary: VcpuStateSnapshotComparison,
}

impl BoundedTwoVcpuCheckpointComparison {
    #[must_use]
    pub const fn vcpu_ids(&self) -> [VcpuId; 2] {
        [self.primary_id, self.secondary_id]
    }

    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPageComparison] {
        self.primary.pages()
    }

    #[must_use]
    pub fn page_exact(&self, address: GuestPhysAddr) -> Option<bool> {
        self.primary.page_exact(address)
    }

    #[must_use]
    pub fn vcpu_exact(&self, id: VcpuId) -> Option<bool> {
        if id == self.primary_id {
            Some(self.primary.vcpu().is_exact_match())
        } else if id == self.secondary_id {
            Some(self.secondary.is_exact_match())
        } else {
            None
        }
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.primary.is_exact_match() && self.secondary.is_exact_match()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedTwoVcpuFullControllerCheckpoint {
    base: BoundedTwoVcpuCheckpoint,
    master_pic: MasterPicStateSnapshot,
    slave_pic: SlavePicStateSnapshot,
    ioapic: IoapicStateSnapshot,
    lapics: [(VcpuId, KvmLapicState); 2],
    mp_states: [(VcpuId, u32); 2],
}

impl BoundedTwoVcpuFullControllerCheckpoint {
    pub fn capture(
        first: &Vcpu,
        second: &Vcpu,
        vm: &Vm,
        msr_policy: &GuestMsrAccessPolicy,
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let memory = vm.guest_memory().ok_or_else(|| {
            two_vcpu_checkpoint_error(
                VcpuId::BOOT,
                "two-vCPU full-controller checkpoint capture",
                "VM has no registered guest memory",
            )
        })?;
        let base =
            BoundedTwoVcpuCheckpoint::capture(first, second, msr_policy, memory, page_addresses)?;
        let (primary, secondary) = canonical_vcpu_pair(first, second)?;
        Ok(Self {
            base,
            master_pic: vm.capture_master_pic_state()?,
            slave_pic: vm.capture_slave_pic_state()?,
            ioapic: vm.capture_ioapic_state()?,
            lapics: [
                (primary.id(), primary.capture_lapic_checkpoint_state()?),
                (secondary.id(), secondary.capture_lapic_checkpoint_state()?),
            ],
            mp_states: [
                (primary.id(), primary.multiprocessing_state_raw()?),
                (secondary.id(), secondary.multiprocessing_state_raw()?),
            ],
        })
    }

    #[must_use]
    pub const fn vcpu_ids(&self) -> [VcpuId; 2] {
        self.base.vcpu_ids()
    }

    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPage] {
        self.base.pages()
    }

    pub fn verify(
        &self,
        first: &Vcpu,
        second: &Vcpu,
        vm: &Vm,
    ) -> Result<BoundedTwoVcpuFullControllerCheckpointComparison, Error> {
        let memory = vm.guest_memory().ok_or_else(|| {
            two_vcpu_checkpoint_error(
                VcpuId::BOOT,
                "two-vCPU full-controller checkpoint verify",
                "VM has no registered guest memory",
            )
        })?;
        let base = self.base.verify(first, second, memory)?;
        let (primary, secondary) = self.bind_vcpus(first, second)?;
        Ok(BoundedTwoVcpuFullControllerCheckpointComparison {
            base,
            master_pic: vm.capture_master_pic_state()? == self.master_pic,
            slave_pic: vm.capture_slave_pic_state()? == self.slave_pic,
            ioapic: vm.capture_ioapic_state()? == self.ioapic,
            lapics: [
                primary.capture_lapic_checkpoint_state()? == self.lapics[0].1,
                secondary.capture_lapic_checkpoint_state()? == self.lapics[1].1,
            ],
            mp_states: [
                primary.multiprocessing_state_raw()? == self.mp_states[0].1,
                secondary.multiprocessing_state_raw()? == self.mp_states[1].1,
            ],
        })
    }

    pub fn restore_and_verify(
        &self,
        first: &mut Vcpu,
        second: &mut Vcpu,
        vm: &mut Vm,
    ) -> Result<BoundedTwoVcpuFullControllerCheckpointComparison, Error> {
        let (primary, secondary) = canonical_vcpu_pair_mut(first, second)?;
        self.require_bound_vcpus(primary, secondary)?;
        {
            let memory = vm.guest_memory_mut().ok_or_else(|| {
                two_vcpu_checkpoint_error(
                    VcpuId::BOOT,
                    "two-vCPU full-controller checkpoint restore",
                    "VM has no registered guest memory",
                )
            })?;
            let base = self.base.restore_and_verify(primary, secondary, memory)?;
            if !base.is_exact_match() {
                return Err(two_vcpu_checkpoint_error(
                    primary.id(),
                    "two-vCPU full-controller guest restore verification",
                    "page/vCPU state was not exact; MP/controller restore was not attempted",
                ));
            }
        }

        let primary_mp = primary.restore_multiprocessing_state_raw(self.mp_states[0].1)?;
        let secondary_mp = secondary.restore_multiprocessing_state_raw(self.mp_states[1].1)?;
        if primary_mp != self.mp_states[0].1 || secondary_mp != self.mp_states[1].1 {
            return Err(two_vcpu_checkpoint_error(
                primary.id(),
                "two-vCPU full-controller MP-state restore verification",
                format!(
                    "expected MP states [{}, {}], got [{primary_mp}, {secondary_mp}]",
                    self.mp_states[0].1, self.mp_states[1].1
                ),
            ));
        }

        vm.restore_master_pic_state(&self.master_pic)?;
        vm.restore_slave_pic_state(&self.slave_pic)?;
        vm.restore_ioapic_state(&self.ioapic)?;
        primary.restore_lapic_checkpoint_state(&self.lapics[0].1)?;
        secondary.restore_lapic_checkpoint_state(&self.lapics[1].1)?;
        self.verify(primary, secondary, vm)
    }

    fn bind_vcpus<'a>(
        &self,
        first: &'a Vcpu,
        second: &'a Vcpu,
    ) -> Result<(&'a Vcpu, &'a Vcpu), Error> {
        let (primary, secondary) = canonical_vcpu_pair(first, second)?;
        self.require_bound_vcpus(primary, secondary)?;
        Ok((primary, secondary))
    }

    fn require_bound_vcpus(&self, primary: &Vcpu, secondary: &Vcpu) -> Result<(), Error> {
        if [primary.id(), secondary.id()] != self.vcpu_ids()
            || self.lapics[0].0 != primary.id()
            || self.lapics[1].0 != secondary.id()
            || self.mp_states[0].0 != primary.id()
            || self.mp_states[1].0 != secondary.id()
        {
            return Err(two_vcpu_checkpoint_error(
                primary.id(),
                "two-vCPU full-controller checkpoint binding",
                format!(
                    "checkpoint owns vCPUs {:?}, LAPICs [{}, {}], MP states [{}, {}]; supplied [{}, {}]",
                    self.vcpu_ids(),
                    self.lapics[0].0.get(),
                    self.lapics[1].0.get(),
                    self.mp_states[0].0.get(),
                    self.mp_states[1].0.get(),
                    primary.id().get(),
                    secondary.id().get()
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedTwoVcpuFullControllerCheckpointComparison {
    base: BoundedTwoVcpuCheckpointComparison,
    master_pic: bool,
    slave_pic: bool,
    ioapic: bool,
    lapics: [bool; 2],
    mp_states: [bool; 2],
}

impl BoundedTwoVcpuFullControllerCheckpointComparison {
    #[must_use]
    pub fn page_exact(&self, address: GuestPhysAddr) -> Option<bool> {
        self.base.page_exact(address)
    }

    #[must_use]
    pub fn vcpu_exact(&self, id: VcpuId) -> Option<bool> {
        self.base.vcpu_exact(id)
    }

    #[must_use]
    pub const fn master_pic_exact(&self) -> bool {
        self.master_pic
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
    pub fn lapic_exact(&self, id: VcpuId) -> Option<bool> {
        let ids = self.base.vcpu_ids();
        if id == ids[0] {
            Some(self.lapics[0])
        } else if id == ids[1] {
            Some(self.lapics[1])
        } else {
            None
        }
    }

    #[must_use]
    pub fn mp_state_exact(&self, id: VcpuId) -> Option<bool> {
        let ids = self.base.vcpu_ids();
        if id == ids[0] {
            Some(self.mp_states[0])
        } else if id == ids[1] {
            Some(self.mp_states[1])
        } else {
            None
        }
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.base.is_exact_match()
            && self.master_pic
            && self.slave_pic
            && self.ioapic
            && self.lapics == [true, true]
            && self.mp_states == [true, true]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuFullControllerCheckpointGuestResult {
    first_capture_rip: u64,
    second_capture_rip: u64,
    first_capture_rflags: u64,
    second_capture_rflags: u64,
    captured_pages: Vec<GuestPhysAddr>,
    corruption: BoundedTwoVcpuFullControllerCheckpointComparison,
    restored: BoundedTwoVcpuFullControllerCheckpointComparison,
    first_proof: Vec<u8>,
    second_proof: Vec<u8>,
    first_completion_rip: u64,
    second_completion_rip: u64,
    first_completion_rflags: u64,
    second_completion_rflags: u64,
}

impl TwoVcpuFullControllerCheckpointGuestResult {
    #[must_use]
    pub const fn first_capture_rip(&self) -> u64 {
        self.first_capture_rip
    }

    #[must_use]
    pub const fn second_capture_rip(&self) -> u64 {
        self.second_capture_rip
    }

    #[must_use]
    pub const fn first_capture_rflags(&self) -> u64 {
        self.first_capture_rflags
    }

    #[must_use]
    pub const fn second_capture_rflags(&self) -> u64 {
        self.second_capture_rflags
    }

    #[must_use]
    pub fn captured_pages(&self) -> &[GuestPhysAddr] {
        &self.captured_pages
    }

    #[must_use]
    pub const fn corruption(&self) -> &BoundedTwoVcpuFullControllerCheckpointComparison {
        &self.corruption
    }

    #[must_use]
    pub const fn restored(&self) -> &BoundedTwoVcpuFullControllerCheckpointComparison {
        &self.restored
    }

    #[must_use]
    pub fn first_proof(&self) -> &[u8] {
        &self.first_proof
    }

    #[must_use]
    pub fn second_proof(&self) -> &[u8] {
        &self.second_proof
    }

    #[must_use]
    pub const fn first_completion_rip(&self) -> u64 {
        self.first_completion_rip
    }

    #[must_use]
    pub const fn second_completion_rip(&self) -> u64 {
        self.second_completion_rip
    }

    #[must_use]
    pub const fn first_completion_rflags(&self) -> u64 {
        self.first_completion_rflags
    }

    #[must_use]
    pub const fn second_completion_rflags(&self) -> u64 {
        self.second_completion_rflags
    }
}

pub fn run_two_vcpu_full_controller_checkpoint_guest(
) -> Result<TwoVcpuFullControllerCheckpointGuestResult, Error> {
    let first_image = FlatGuestImage::new(
        TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
        TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
        &TWO_VCPU_FULL_CONTROLLER_FIRST_GUEST_BYTES,
    )?;
    let second_image = FlatGuestImage::new(
        TWO_VCPU_CHECKPOINT_SECOND_ENTRY,
        TWO_VCPU_CHECKPOINT_SECOND_ENTRY,
        &TWO_VCPU_FULL_CONTROLLER_SECOND_GUEST_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    backend.require_mp_state_capability()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let first_layout = LongModeBootLayout::new(
        memory.region(),
        first_image.entry(),
        TWO_VCPU_CHECKPOINT_FIRST_STACK,
    )
    .expect("fixed first full-controller two-vCPU layout remains valid");
    let second_layout = LongModeBootLayout::new(
        memory.region(),
        second_image.entry(),
        TWO_VCPU_CHECKPOINT_SECOND_STACK,
    )
    .expect("fixed second full-controller two-vCPU layout remains valid");
    let first_corrupt_layout =
        LongModeBootLayout::new(memory.region(), GuestPhysAddr::new(0x12000), 0x1fbff8)
            .expect("fixed first full-controller corruption layout remains valid");
    let second_corrupt_layout =
        LongModeBootLayout::new(memory.region(), GuestPhysAddr::new(0x13000), 0x1faff8)
            .expect("fixed second full-controller corruption layout remains valid");
    first_layout.install_page_tables(&mut memory)?;
    first_image.load(&mut memory)?;
    second_image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    let mut first_vcpu = vm.create_vcpu(TWO_VCPU_CHECKPOINT_FIRST_ID)?;
    let mut second_vcpu = vm.create_vcpu(TWO_VCPU_CHECKPOINT_SECOND_ID)?;
    first_vcpu.initialize_long_mode(&first_layout)?;
    second_vcpu.initialize_long_mode(&second_layout)?;
    let first_mp_state = first_vcpu.ensure_runnable_mp_state()?;
    let second_mp_state = second_vcpu.ensure_runnable_mp_state()?;
    if first_mp_state != TWO_VCPU_FULL_CONTROLLER_MP_STATE_RUNNABLE
        || second_mp_state != TWO_VCPU_FULL_CONTROLLER_MP_STATE_RUNNABLE
    {
        return Err(two_vcpu_checkpoint_error(
            TWO_VCPU_CHECKPOINT_FIRST_ID,
            "two-vCPU full-controller runnable MP-state preparation",
            format!("expected [0, 0], got [{first_mp_state}, {second_mp_state}]"),
        ));
    }
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty full-controller two-vCPU checkpoint MSR policy is valid");

    let (first_capture_rip, first_capture_rflags) = run_full_controller_debug_barrier(
        &mut first_vcpu,
        TWO_VCPU_FULL_CONTROLLER_FIRST_CAPTURE_MARKER,
        TWO_VCPU_FULL_CONTROLLER_FIRST_CAPTURE_RIP,
        "first full-controller two-vCPU checkpoint quiescence",
    )?;
    let (second_capture_rip, second_capture_rflags) = run_full_controller_debug_barrier(
        &mut second_vcpu,
        TWO_VCPU_FULL_CONTROLLER_SECOND_CAPTURE_MARKER,
        TWO_VCPU_FULL_CONTROLLER_SECOND_CAPTURE_RIP,
        "second full-controller two-vCPU checkpoint quiescence",
    )?;

    let checkpoint = BoundedTwoVcpuFullControllerCheckpoint::capture(
        &first_vcpu,
        &second_vcpu,
        &vm,
        &msr_policy,
        &TWO_VCPU_CHECKPOINT_OWNERSHIP_SET,
    )?;
    require_captured_roles(&checkpoint.base)?;
    let captured_pages = checkpoint
        .pages()
        .iter()
        .map(BoundedCheckpointPage::address)
        .collect::<Vec<_>>();

    corrupt_owned_pages(
        vm.guest_memory_mut()
            .expect("registered full-controller two-vCPU memory remains VM-owned"),
    )?;
    first_vcpu.initialize_long_mode(&first_corrupt_layout)?;
    second_vcpu.initialize_long_mode(&second_corrupt_layout)?;
    first_vcpu.restore_multiprocessing_state_raw(TWO_VCPU_FULL_CONTROLLER_MP_STATE_HALTED)?;
    second_vcpu
        .restore_multiprocessing_state_raw(TWO_VCPU_FULL_CONTROLLER_MP_STATE_UNINITIALIZED)?;
    corrupt_full_controller_state(&checkpoint, &first_vcpu, &second_vcpu, &vm)?;

    let corruption = checkpoint.verify(&first_vcpu, &second_vcpu, &vm)?;
    require_full_controller_mismatch(&corruption)?;

    let restored = checkpoint.restore_and_verify(&mut first_vcpu, &mut second_vcpu, &mut vm)?;
    if !restored.is_exact_match() {
        return Err(two_vcpu_checkpoint_error(
            TWO_VCPU_CHECKPOINT_FIRST_ID,
            "two-vCPU full-controller checkpoint restore verification",
            format!(
                "restore mismatch: vcpu0={:?} vcpu1={:?} mp0={:?} mp1={:?} master={} slave={} ioapic={} lapic0={:?} lapic1={:?}",
                restored.vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                restored.vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                restored.mp_state_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                restored.mp_state_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                restored.master_pic_exact(),
                restored.slave_pic_exact(),
                restored.ioapic_exact(),
                restored.lapic_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                restored.lapic_exact(TWO_VCPU_CHECKPOINT_SECOND_ID)
            ),
        ));
    }

    let (first_completion_rip, first_completion_rflags) = run_full_controller_debug_barrier(
        &mut first_vcpu,
        TWO_VCPU_CHECKPOINT_FIRST_PROOF[0],
        TWO_VCPU_FULL_CONTROLLER_FIRST_COMPLETION_RIP,
        "first full-controller two-vCPU checkpoint resume",
    )?;
    let first_proof = TWO_VCPU_CHECKPOINT_FIRST_PROOF.to_vec();
    let (second_completion_rip, second_completion_rflags) = run_full_controller_debug_barrier(
        &mut second_vcpu,
        TWO_VCPU_CHECKPOINT_SECOND_PROOF[0],
        TWO_VCPU_FULL_CONTROLLER_SECOND_COMPLETION_RIP,
        "second full-controller two-vCPU checkpoint resume",
    )?;
    let second_proof = TWO_VCPU_CHECKPOINT_SECOND_PROOF.to_vec();

    Ok(TwoVcpuFullControllerCheckpointGuestResult {
        first_capture_rip,
        second_capture_rip,
        first_capture_rflags,
        second_capture_rflags,
        captured_pages,
        corruption,
        restored,
        first_proof,
        second_proof,
        first_completion_rip,
        second_completion_rip,
        first_completion_rflags,
        second_completion_rflags,
    })
}

fn corrupt_full_controller_state(
    checkpoint: &BoundedTwoVcpuFullControllerCheckpoint,
    first: &Vcpu,
    second: &Vcpu,
    vm: &Vm,
) -> Result<(), Error> {
    vm.restore_master_pic_state(
        &checkpoint
            .master_pic
            .with_imr(checkpoint.master_pic.imr() ^ 0x01),
    )?;
    vm.restore_slave_pic_state(
        &checkpoint
            .slave_pic
            .with_imr(checkpoint.slave_pic.imr() ^ 0x02),
    )?;

    let ioapic_entry = checkpoint
        .ioapic
        .redirection_entry(TWO_VCPU_FULL_CONTROLLER_IOAPIC_PIN)
        .expect("fixed full-controller two-vCPU IOAPIC pin remains valid");
    let corrupt_ioapic = checkpoint
        .ioapic
        .with_redirection_entry(
            TWO_VCPU_FULL_CONTROLLER_IOAPIC_PIN,
            ioapic_entry ^ (1_u64 << 16),
        )
        .expect("fixed full-controller two-vCPU IOAPIC pin remains valid");
    vm.restore_ioapic_state(&corrupt_ioapic)?;

    let mut first_lapic = checkpoint.lapics[0].1.clone();
    let first_lvt0 =
        two_vcpu_read_lapic_register(&first_lapic, TWO_VCPU_FULL_CONTROLLER_APIC_LVT0_OFFSET);
    two_vcpu_write_lapic_register(
        &mut first_lapic,
        TWO_VCPU_FULL_CONTROLLER_APIC_LVT0_OFFSET,
        first_lvt0 ^ TWO_VCPU_FULL_CONTROLLER_APIC_LVT_MASKED,
    );
    first.restore_lapic_checkpoint_state(&first_lapic)?;

    let mut second_lapic = checkpoint.lapics[1].1.clone();
    let second_spiv =
        two_vcpu_read_lapic_register(&second_lapic, TWO_VCPU_FULL_CONTROLLER_APIC_SPIV_OFFSET);
    two_vcpu_write_lapic_register(
        &mut second_lapic,
        TWO_VCPU_FULL_CONTROLLER_APIC_SPIV_OFFSET,
        second_spiv ^ TWO_VCPU_FULL_CONTROLLER_APIC_SOFTWARE_ENABLE,
    );
    second.restore_lapic_checkpoint_state(&second_lapic)?;
    Ok(())
}

fn require_full_controller_mismatch(
    comparison: &BoundedTwoVcpuFullControllerCheckpointComparison,
) -> Result<(), Error> {
    for address in TWO_VCPU_CHECKPOINT_OWNERSHIP_SET {
        if comparison.page_exact(address) != Some(false) {
            return Err(two_vcpu_checkpoint_error(
                TWO_VCPU_CHECKPOINT_FIRST_ID,
                "two-vCPU full-controller checkpoint corruption proof",
                format!("owned page {:#x} did not mismatch", address.get()),
            ));
        }
    }
    if comparison.vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID) != Some(false)
        || comparison.vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID) != Some(false)
        || comparison.mp_state_exact(TWO_VCPU_CHECKPOINT_FIRST_ID) != Some(false)
        || comparison.mp_state_exact(TWO_VCPU_CHECKPOINT_SECOND_ID) != Some(false)
        || comparison.master_pic_exact()
        || comparison.slave_pic_exact()
        || comparison.ioapic_exact()
        || comparison.lapic_exact(TWO_VCPU_CHECKPOINT_FIRST_ID) != Some(false)
        || comparison.lapic_exact(TWO_VCPU_CHECKPOINT_SECOND_ID) != Some(false)
    {
        return Err(two_vcpu_checkpoint_error(
            TWO_VCPU_CHECKPOINT_FIRST_ID,
            "two-vCPU full-controller checkpoint corruption proof",
            format!(
                "expected vCPU/MP/controller/LAPIC mismatches, got vcpu0={:?} vcpu1={:?} mp0={:?} mp1={:?} master={} slave={} ioapic={} lapic0={:?} lapic1={:?}",
                comparison.vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                comparison.vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                comparison.mp_state_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                comparison.mp_state_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                comparison.master_pic_exact(),
                comparison.slave_pic_exact(),
                comparison.ioapic_exact(),
                comparison.lapic_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                comparison.lapic_exact(TWO_VCPU_CHECKPOINT_SECOND_ID)
            ),
        ));
    }
    Ok(())
}

fn two_vcpu_read_lapic_register(state: &KvmLapicState, offset: usize) -> u32 {
    u32::from_le_bytes(
        state.regs[offset..offset + 4]
            .try_into()
            .expect("fixed LAPIC register offset remains valid"),
    )
}

fn two_vcpu_write_lapic_register(state: &mut KvmLapicState, offset: usize, value: u32) {
    state.regs[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuCheckpointGuestResult {
    first_capture: VmExitReport,
    second_capture: VmExitReport,
    captured_pages: Vec<GuestPhysAddr>,
    corruption: BoundedTwoVcpuCheckpointComparison,
    restored: BoundedTwoVcpuCheckpointComparison,
    first_io_exits: Vec<PortIoExit>,
    second_io_exits: Vec<PortIoExit>,
    first_proof: Vec<u8>,
    second_proof: Vec<u8>,
    first_terminal: VmExitReport,
    second_terminal: VmExitReport,
}

impl TwoVcpuCheckpointGuestResult {
    #[must_use]
    pub const fn first_capture(&self) -> VmExitReport {
        self.first_capture
    }
    #[must_use]
    pub const fn second_capture(&self) -> VmExitReport {
        self.second_capture
    }
    #[must_use]
    pub fn captured_pages(&self) -> &[GuestPhysAddr] {
        &self.captured_pages
    }
    #[must_use]
    pub const fn corruption(&self) -> &BoundedTwoVcpuCheckpointComparison {
        &self.corruption
    }
    #[must_use]
    pub const fn restored(&self) -> &BoundedTwoVcpuCheckpointComparison {
        &self.restored
    }
    #[must_use]
    pub fn first_io_exits(&self) -> &[PortIoExit] {
        &self.first_io_exits
    }
    #[must_use]
    pub fn second_io_exits(&self) -> &[PortIoExit] {
        &self.second_io_exits
    }
    #[must_use]
    pub fn first_proof(&self) -> &[u8] {
        &self.first_proof
    }
    #[must_use]
    pub fn second_proof(&self) -> &[u8] {
        &self.second_proof
    }
    #[must_use]
    pub const fn first_terminal(&self) -> VmExitReport {
        self.first_terminal
    }
    #[must_use]
    pub const fn second_terminal(&self) -> VmExitReport {
        self.second_terminal
    }
}

pub fn run_two_vcpu_checkpoint_guest() -> Result<TwoVcpuCheckpointGuestResult, Error> {
    let first_image = FlatGuestImage::new(
        TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
        TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
        &FIRST_GUEST_BYTES,
    )?;
    let second_image = FlatGuestImage::new(
        TWO_VCPU_CHECKPOINT_SECOND_ENTRY,
        TWO_VCPU_CHECKPOINT_SECOND_ENTRY,
        &SECOND_GUEST_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let first_layout = LongModeBootLayout::new(
        memory.region(),
        first_image.entry(),
        TWO_VCPU_CHECKPOINT_FIRST_STACK,
    )
    .expect("fixed first two-vCPU checkpoint layout remains valid");
    let second_layout = LongModeBootLayout::new(
        memory.region(),
        second_image.entry(),
        TWO_VCPU_CHECKPOINT_SECOND_STACK,
    )
    .expect("fixed second two-vCPU checkpoint layout remains valid");
    let first_corrupt_layout =
        LongModeBootLayout::new(memory.region(), GuestPhysAddr::new(0x12000), 0x1fbff8)
            .expect("fixed first corruption layout remains valid");
    let second_corrupt_layout =
        LongModeBootLayout::new(memory.region(), GuestPhysAddr::new(0x13000), 0x1faff8)
            .expect("fixed second corruption layout remains valid");
    first_layout.install_page_tables(&mut memory)?;
    first_image.load(&mut memory)?;
    second_image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    let mut first_vcpu = vm.create_vcpu(TWO_VCPU_CHECKPOINT_FIRST_ID)?;
    let mut second_vcpu = vm.create_vcpu(TWO_VCPU_CHECKPOINT_SECOND_ID)?;
    first_vcpu.initialize_long_mode(&first_layout)?;
    second_vcpu.initialize_long_mode(&second_layout)?;
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty two-vCPU checkpoint MSR policy is valid by construction");

    let first_capture = run_to_quiescent_hlt(
        &mut first_vcpu,
        TWO_VCPU_CHECKPOINT_FIRST_CAPTURE_RIP,
        "first two-vCPU checkpoint quiescence",
    )?;
    let second_capture = run_to_quiescent_hlt(
        &mut second_vcpu,
        TWO_VCPU_CHECKPOINT_SECOND_CAPTURE_RIP,
        "second two-vCPU checkpoint quiescence",
    )?;

    let checkpoint = BoundedTwoVcpuCheckpoint::capture(
        &first_vcpu,
        &second_vcpu,
        &msr_policy,
        vm.guest_memory()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
        &TWO_VCPU_CHECKPOINT_OWNERSHIP_SET,
    )?;
    require_captured_roles(&checkpoint)?;
    let captured_pages = checkpoint
        .pages()
        .iter()
        .map(BoundedCheckpointPage::address)
        .collect::<Vec<_>>();

    corrupt_owned_pages(
        vm.guest_memory_mut()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
    )?;
    first_vcpu.initialize_long_mode(&first_corrupt_layout)?;
    second_vcpu.initialize_long_mode(&second_corrupt_layout)?;

    let corruption = checkpoint.verify(
        &first_vcpu,
        &second_vcpu,
        vm.guest_memory()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
    )?;
    require_full_mismatch("two-vCPU checkpoint corruption proof", &corruption)?;

    let restored = checkpoint.restore_and_verify(
        &first_vcpu,
        &second_vcpu,
        vm.guest_memory_mut()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
    )?;
    if !restored.is_exact_match() {
        return Err(two_vcpu_checkpoint_error(
            TWO_VCPU_CHECKPOINT_FIRST_ID,
            "two-vCPU checkpoint restore verification",
            format!(
                "restore mismatch: pages={:?}, vcpu0={:?}, vcpu1={:?}",
                restored.pages(),
                restored.vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                restored.vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID)
            ),
        ));
    }

    let (first_io_exits, first_proof, first_terminal) = resume_and_verify(
        &mut first_vcpu,
        TWO_VCPU_CHECKPOINT_FIRST_PROOF,
        TWO_VCPU_CHECKPOINT_FIRST_TERMINAL_RIP,
        "first two-vCPU checkpoint resume",
    )?;
    let (second_io_exits, second_proof, second_terminal) = resume_and_verify(
        &mut second_vcpu,
        TWO_VCPU_CHECKPOINT_SECOND_PROOF,
        TWO_VCPU_CHECKPOINT_SECOND_TERMINAL_RIP,
        "second two-vCPU checkpoint resume",
    )?;

    Ok(TwoVcpuCheckpointGuestResult {
        first_capture,
        second_capture,
        captured_pages,
        corruption,
        restored,
        first_io_exits,
        second_io_exits,
        first_proof,
        second_proof,
        first_terminal,
        second_terminal,
    })
}

fn canonical_vcpu_pair<'a>(
    first: &'a Vcpu,
    second: &'a Vcpu,
) -> Result<(&'a Vcpu, &'a Vcpu), Error> {
    if first.id() == second.id() {
        return Err(two_vcpu_checkpoint_error(
            first.id(),
            "two-vCPU checkpoint ownership",
            "checkpoint requires two distinct vCPU ids",
        ));
    }
    if first.id().get() < second.id().get() {
        Ok((first, second))
    } else {
        Ok((second, first))
    }
}

fn canonical_vcpu_pair_mut<'a>(
    first: &'a mut Vcpu,
    second: &'a mut Vcpu,
) -> Result<(&'a mut Vcpu, &'a mut Vcpu), Error> {
    if first.id() == second.id() {
        return Err(two_vcpu_checkpoint_error(
            first.id(),
            "two-vCPU checkpoint mutable ownership",
            "checkpoint requires two distinct vCPU ids",
        ));
    }
    if first.id().get() < second.id().get() {
        Ok((first, second))
    } else {
        Ok((second, first))
    }
}

fn run_full_controller_debug_barrier(
    vcpu: &mut Vcpu,
    expected: u8,
    expected_rip: u64,
    operation: &'static str,
) -> Result<(u64, u64), Error> {
    let mut port_io = PortIoBus::with_debug_port();
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Io {
        return Err(two_vcpu_checkpoint_error(
            vcpu.id(),
            operation,
            format!("expected KVM_EXIT_IO marker, got {exit:?}"),
        ));
    }
    let io_exit = vcpu.port_io_exit()?;
    if io_exit.direction() != PortIoDirection::Out
        || io_exit.port() != DEBUG_PORT
        || io_exit.size() != 1
        || io_exit.count() != 1
        || io_exit.output_data() != [expected]
    {
        return Err(two_vcpu_checkpoint_error(
            vcpu.id(),
            operation,
            format!("unexpected debug output {io_exit:?}; expected {expected:#x}"),
        ));
    }
    if port_io.dispatch(&io_exit)? != PortIoService::Output {
        return Err(two_vcpu_checkpoint_error(
            vcpu.id(),
            operation,
            "debug marker unexpectedly requested an input response",
        ));
    }

    // KVM_EXIT_IO exposes an operation whose userspace completion is finalized only when KVM_RUN
    // is re-entered. Never checkpoint that intermediate kvm_run state. Single-step the dedicated
    // adjacent NOP so the marker I/O retires first and KVM returns KVM_EXIT_DEBUG at a fully
    // architectural boundary while the stack marker remains untouched.
    vcpu.set_guest_single_step(true)?;
    let step_result = vcpu.run_once();
    let disable_result = vcpu.set_guest_single_step(false);
    let step_exit = match (step_result, disable_result) {
        (Ok(exit), Ok(())) => exit,
        (Err(error), _) | (Ok(_), Err(error)) => return Err(error),
    };
    if step_exit != VcpuExit::Debug {
        return Err(two_vcpu_checkpoint_error(
            vcpu.id(),
            operation,
            format!("expected KVM_EXIT_DEBUG after retiring marker I/O, got {step_exit:?}"),
        ));
    }

    let registers = vcpu.registers()?;
    if registers.rip != expected_rip || registers.rflags & 0x2 != 0x2 {
        return Err(two_vcpu_checkpoint_error(
            vcpu.id(),
            operation,
            format!(
                "expected retired checkpoint RIP {expected_rip:#x} with architectural bit1, got rip={:#x} rflags={:#x}",
                registers.rip, registers.rflags
            ),
        ));
    }
    Ok((registers.rip, registers.rflags))
}

fn run_to_quiescent_hlt(
    vcpu: &mut Vcpu,
    expected_rip: u64,
    operation: &'static str,
) -> Result<VmExitReport, Error> {
    let mut no_io = PortIoBus::empty();
    let execution = run_vcpu_until_stopped(vcpu, &mut no_io, 1)?;
    if !execution.io_exits().is_empty() {
        return Err(two_vcpu_checkpoint_error(
            vcpu.id(),
            operation,
            "quiescence boundary unexpectedly serviced port I/O",
        ));
    }
    require_hlt_report(vcpu.id(), operation, execution.report(), expected_rip)?;
    Ok(execution.report())
}

fn require_captured_roles(checkpoint: &BoundedTwoVcpuCheckpoint) -> Result<(), Error> {
    let page = |address| {
        checkpoint
            .pages()
            .iter()
            .find(|page| page.address() == address)
    };
    let shared = page(TWO_VCPU_CHECKPOINT_SHARED_PAGE)
        .and_then(|page| page.bytes().first())
        .copied();
    let first_offset = usize::try_from(
        TWO_VCPU_CHECKPOINT_FIRST_STACK - 8 - TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE.get(),
    )
    .expect("fixed first stack marker offset fits usize");
    let second_offset = usize::try_from(
        TWO_VCPU_CHECKPOINT_SECOND_STACK - 8 - TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE.get(),
    )
    .expect("fixed second stack marker offset fits usize");
    let first_stack = page(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE)
        .and_then(|page| page.bytes().get(first_offset))
        .copied();
    let second_stack = page(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE)
        .and_then(|page| page.bytes().get(second_offset))
        .copied();
    if shared != Some(TWO_VCPU_CHECKPOINT_SHARED_MARKER)
        || first_stack != Some(TWO_VCPU_CHECKPOINT_FIRST_MARKER)
        || second_stack != Some(TWO_VCPU_CHECKPOINT_SECOND_MARKER)
    {
        return Err(two_vcpu_checkpoint_error(
            TWO_VCPU_CHECKPOINT_FIRST_ID,
            "two-vCPU checkpoint role capture",
            format!(
                "unexpected roles: shared={shared:?}, first_stack={first_stack:?}, second_stack={second_stack:?}"
            ),
        ));
    }
    Ok(())
}

fn corrupt_owned_pages(memory: &mut GuestMemory) -> Result<(), Error> {
    memory.write(
        TWO_VCPU_CHECKPOINT_SHARED_PAGE,
        &vec![0xa5; LONG_MODE_PAGE_SIZE as usize],
    )?;
    memory.write(
        TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE,
        &vec![0x5a; LONG_MODE_PAGE_SIZE as usize],
    )?;
    memory.write(
        TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE,
        &vec![0xcc; LONG_MODE_PAGE_SIZE as usize],
    )?;
    Ok(())
}

fn require_full_mismatch(
    operation: &'static str,
    comparison: &BoundedTwoVcpuCheckpointComparison,
) -> Result<(), Error> {
    for address in TWO_VCPU_CHECKPOINT_OWNERSHIP_SET {
        if comparison.page_exact(address) != Some(false) {
            return Err(two_vcpu_checkpoint_error(
                TWO_VCPU_CHECKPOINT_FIRST_ID,
                operation,
                format!(
                    "owned page {:#x} did not independently mismatch",
                    address.get()
                ),
            ));
        }
    }
    for id in [TWO_VCPU_CHECKPOINT_FIRST_ID, TWO_VCPU_CHECKPOINT_SECOND_ID] {
        if comparison.vcpu_exact(id) != Some(false) {
            return Err(two_vcpu_checkpoint_error(
                id,
                operation,
                "intentional vCPU corruption remained exact",
            ));
        }
    }
    Ok(())
}

fn resume_and_verify(
    vcpu: &mut Vcpu,
    expected_proof: &[u8],
    expected_rip: u64,
    operation: &'static str,
) -> Result<(Vec<PortIoExit>, Vec<u8>, VmExitReport), Error> {
    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(vcpu, &mut port_io, 2)?;
    require_hlt_report(vcpu.id(), operation, execution.report(), expected_rip)?;
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != expected_proof || execution.io_exits().len() != expected_proof.len() {
        return Err(two_vcpu_checkpoint_error(
            vcpu.id(),
            operation,
            format!("expected proof {expected_proof:?}, got {proof:?}"),
        ));
    }
    for (io, expected) in execution
        .io_exits()
        .iter()
        .zip(expected_proof.iter().copied())
    {
        if io.direction() != PortIoDirection::Out
            || io.port() != DEBUG_PORT
            || io.size() != 1
            || io.count() != 1
            || io.output_data() != [expected]
        {
            return Err(two_vcpu_checkpoint_error(
                vcpu.id(),
                operation,
                format!("unexpected debug-port exit {io:?}"),
            ));
        }
    }
    Ok((execution.io_exits().to_vec(), proof, execution.report()))
}

fn require_hlt_report(
    id: VcpuId,
    operation: &'static str,
    report: VmExitReport,
    expected_rip: u64,
) -> Result<(), Error> {
    if report.exit() != VcpuExit::Hlt
        || report.vcpu_id() != id
        || report.rip() != expected_rip
        || report.rflags() & 0x2 != 0x2
    {
        return Err(two_vcpu_checkpoint_error(
            id,
            operation,
            format!(
                "expected vCPU {} HLT at rip={expected_rip:#x}, got {report}",
                id.get()
            ),
        ));
    }
    Ok(())
}

fn two_vcpu_checkpoint_error(
    id: VcpuId,
    operation: &'static str,
    detail: impl Into<String>,
) -> Error {
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
    fn deterministic_guests_bind_two_quiescent_boundaries_to_two_resume_proofs() {
        assert_eq!(FIRST_GUEST_BYTES.len(), 37);
        assert_eq!(SECOND_GUEST_BYTES.len(), 29);
        assert_eq!(FIRST_GUEST_BYTES[10], 0xf4);
        assert_eq!(SECOND_GUEST_BYTES[2], 0xf4);
        assert_eq!(FIRST_GUEST_BYTES[31], 0xf4);
        assert_eq!(SECOND_GUEST_BYTES[23], 0xf4);
        assert_eq!(
            TWO_VCPU_CHECKPOINT_FIRST_CAPTURE_RIP,
            TWO_VCPU_CHECKPOINT_FIRST_ENTRY.get() + 11
        );
        assert_eq!(
            TWO_VCPU_CHECKPOINT_SECOND_CAPTURE_RIP,
            TWO_VCPU_CHECKPOINT_SECOND_ENTRY.get() + 3
        );
        assert_eq!(
            TWO_VCPU_CHECKPOINT_FIRST_TERMINAL_RIP,
            TWO_VCPU_CHECKPOINT_FIRST_ENTRY.get() + 32
        );
        assert_eq!(
            TWO_VCPU_CHECKPOINT_SECOND_TERMINAL_RIP,
            TWO_VCPU_CHECKPOINT_SECOND_ENTRY.get() + 24
        );
    }

    #[test]
    fn full_controller_guests_use_userspace_barriers_instead_of_hlt_capture() {
        assert_eq!(TWO_VCPU_FULL_CONTROLLER_FIRST_GUEST_BYTES.len(), 42);
        assert_eq!(TWO_VCPU_FULL_CONTROLLER_SECOND_GUEST_BYTES.len(), 34);
        assert_eq!(
            &TWO_VCPU_FULL_CONTROLLER_FIRST_GUEST_BYTES[10..15],
            &[0xb0, b'A', 0xe6, 0xe9, 0x90]
        );
        assert_eq!(
            &TWO_VCPU_FULL_CONTROLLER_SECOND_GUEST_BYTES[2..7],
            &[0xb0, b'B', 0xe6, 0xe9, 0x90]
        );
        assert_eq!(TWO_VCPU_FULL_CONTROLLER_FIRST_CAPTURE_RIP, 0x1000f);
        assert_eq!(TWO_VCPU_FULL_CONTROLLER_SECOND_CAPTURE_RIP, 0x11007);
        assert_eq!(TWO_VCPU_FULL_CONTROLLER_FIRST_COMPLETION_RIP, 0x10024);
        assert_eq!(TWO_VCPU_FULL_CONTROLLER_SECOND_COMPLETION_RIP, 0x1101c);
    }

    #[test]
    fn ownership_set_is_shared_page_plus_both_active_stack_pages() {
        assert_eq!(
            TWO_VCPU_CHECKPOINT_OWNERSHIP_SET,
            [
                GuestPhysAddr::new(0x30000),
                GuestPhysAddr::new(0x1fc000),
                GuestPhysAddr::new(0x1fd000),
            ]
        );
        assert_eq!(TWO_VCPU_CHECKPOINT_FIRST_STACK - 8, 0x1fdff0);
        assert_eq!(TWO_VCPU_CHECKPOINT_SECOND_STACK - 8, 0x1fcff0);
    }
}
