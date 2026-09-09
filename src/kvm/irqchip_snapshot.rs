pub const KVM_GET_IRQCHIP: libc::c_ulong = 0xC208_AE62;
pub const KVM_SET_IRQCHIP: libc::c_ulong = 0x8208_AE63;
const KVM_IRQCHIP_PIC_MASTER: u32 = 0;
const KVM_IRQCHIP_PIC_SLAVE: u32 = 1;
const KVM_IRQCHIP_IOAPIC: u32 = 2;
const KVM_IRQCHIP_PAYLOAD_SIZE: usize = 512;
pub(crate) const KVM_IOAPIC_NUM_PINS: usize = 24;
const KVM_IOAPIC_STATE_SIZE: usize = 216;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KvmPicState {
    last_irr: u8,
    irr: u8,
    imr: u8,
    isr: u8,
    priority_add: u8,
    irq_base: u8,
    read_reg_select: u8,
    poll: u8,
    special_mask: u8,
    init_state: u8,
    auto_eoi: u8,
    rotate_on_auto_eoi: u8,
    special_fully_nested_mode: u8,
    init4: u8,
    elcr: u8,
    elcr_mask: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KvmIoapicState {
    base_address: u64,
    ioregsel: u32,
    id: u32,
    irr: u32,
    pad: u32,
    redirtbl: [u64; KVM_IOAPIC_NUM_PINS],
}

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct KvmIrqchip {
    chip_id: u32,
    pad: u32,
    chip: [u8; KVM_IRQCHIP_PAYLOAD_SIZE],
}

impl KvmIrqchip {
    fn request(chip_id: u32) -> Self {
        Self {
            chip_id,
            pad: 0,
            chip: [0; KVM_IRQCHIP_PAYLOAD_SIZE],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MasterPicStateSnapshot {
    state: KvmPicState,
}

impl MasterPicStateSnapshot {
    #[must_use]
    pub(crate) const fn imr(&self) -> u8 {
        self.state.imr
    }

    #[must_use]
    pub(crate) fn with_imr(&self, imr: u8) -> Self {
        let mut state = self.state;
        state.imr = imr;
        Self { state }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlavePicStateSnapshot {
    state: KvmPicState,
}

impl SlavePicStateSnapshot {
    #[must_use]
    pub(crate) const fn imr(&self) -> u8 {
        self.state.imr
    }

    #[must_use]
    pub(crate) fn with_imr(&self, imr: u8) -> Self {
        let mut state = self.state;
        state.imr = imr;
        Self { state }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IoapicStateSnapshot {
    state: KvmIoapicState,
}

impl IoapicStateSnapshot {
    #[must_use]
    pub(crate) const fn base_address(&self) -> u64 {
        self.state.base_address
    }

    #[must_use]
    pub(crate) const fn ioregsel(&self) -> u32 {
        self.state.ioregsel
    }

    #[must_use]
    pub(crate) const fn id(&self) -> u32 {
        self.state.id
    }

    #[must_use]
    pub(crate) const fn irr(&self) -> u32 {
        self.state.irr
    }

    #[must_use]
    pub(crate) const fn pad(&self) -> u32 {
        self.state.pad
    }

    #[must_use]
    pub(crate) fn redirection_entry(&self, pin: usize) -> Option<u64> {
        self.state.redirtbl.get(pin).copied()
    }

    #[must_use]
    pub(crate) fn with_redirection_entry(&self, pin: usize, entry: u64) -> Option<Self> {
        let mut state = self.state;
        *state.redirtbl.get_mut(pin)? = entry;
        Some(Self { state })
    }
}

impl crate::kvm::Vm {
    pub(crate) fn capture_master_pic_state(
        &self,
    ) -> Result<MasterPicStateSnapshot, crate::error::Error> {
        let request = self.capture_irqchip(KVM_IRQCHIP_PIC_MASTER, "master PIC")?;
        Ok(MasterPicStateSnapshot {
            state: decode_pic_state(&request.chip),
        })
    }

    pub(crate) fn restore_master_pic_state(
        &self,
        snapshot: &MasterPicStateSnapshot,
    ) -> Result<(), crate::error::Error> {
        let mut request = KvmIrqchip::request(KVM_IRQCHIP_PIC_MASTER);
        encode_pic_state(snapshot.state, &mut request.chip);
        self.restore_irqchip(&request, "master PIC")
    }

    pub(crate) fn capture_slave_pic_state(
        &self,
    ) -> Result<SlavePicStateSnapshot, crate::error::Error> {
        let request = self.capture_irqchip(KVM_IRQCHIP_PIC_SLAVE, "slave PIC")?;
        Ok(SlavePicStateSnapshot {
            state: decode_pic_state(&request.chip),
        })
    }

    pub(crate) fn restore_slave_pic_state(
        &self,
        snapshot: &SlavePicStateSnapshot,
    ) -> Result<(), crate::error::Error> {
        let mut request = KvmIrqchip::request(KVM_IRQCHIP_PIC_SLAVE);
        encode_pic_state(snapshot.state, &mut request.chip);
        self.restore_irqchip(&request, "slave PIC")
    }

    pub(crate) fn capture_ioapic_state(
        &self,
    ) -> Result<IoapicStateSnapshot, crate::error::Error> {
        let request = self.capture_irqchip(KVM_IRQCHIP_IOAPIC, "IOAPIC")?;
        Ok(IoapicStateSnapshot {
            state: decode_ioapic_state(&request.chip),
        })
    }

    pub(crate) fn restore_ioapic_state(
        &self,
        snapshot: &IoapicStateSnapshot,
    ) -> Result<(), crate::error::Error> {
        let mut request = KvmIrqchip::request(KVM_IRQCHIP_IOAPIC);
        encode_ioapic_state(snapshot.state, &mut request.chip);
        self.restore_irqchip(&request, "IOAPIC")
    }

    fn capture_irqchip(
        &self,
        chip_id: u32,
        chip_name: &'static str,
    ) -> Result<KvmIrqchip, crate::error::Error> {
        let fd = std::os::fd::AsRawFd::as_raw_fd(&self.fd);
        let mut request = KvmIrqchip::request(chip_id);
        get_irqchip(fd, &mut request)
            .map_err(|source| irqchip_operation_error("KVM_GET_IRQCHIP", chip_name, source))?;
        if request.chip_id != chip_id || request.pad != 0 {
            return Err(irqchip_operation_error(
                "validate KVM_GET_IRQCHIP",
                chip_name,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "expected chip_id {chip_id} and zero outer pad, got chip_id {} pad {}",
                        request.chip_id, request.pad
                    ),
                ),
            ));
        }
        Ok(request)
    }

    fn restore_irqchip(
        &self,
        request: &KvmIrqchip,
        chip_name: &'static str,
    ) -> Result<(), crate::error::Error> {
        let fd = std::os::fd::AsRawFd::as_raw_fd(&self.fd);
        set_irqchip(fd, request)
            .map_err(|source| irqchip_operation_error("KVM_SET_IRQCHIP", chip_name, source))
    }
}

fn irqchip_operation_error(
    operation: &'static str,
    chip_name: &'static str,
    source: std::io::Error,
) -> crate::error::Error {
    let operation = match (operation, chip_name) {
        ("KVM_GET_IRQCHIP", "master PIC") => "KVM_GET_IRQCHIP master PIC",
        ("KVM_SET_IRQCHIP", "master PIC") => "KVM_SET_IRQCHIP master PIC",
        ("validate KVM_GET_IRQCHIP", "master PIC") => "validate KVM_GET_IRQCHIP master PIC",
        ("KVM_GET_IRQCHIP", "slave PIC") => "KVM_GET_IRQCHIP slave PIC",
        ("KVM_SET_IRQCHIP", "slave PIC") => "KVM_SET_IRQCHIP slave PIC",
        ("validate KVM_GET_IRQCHIP", "slave PIC") => "validate KVM_GET_IRQCHIP slave PIC",
        ("KVM_GET_IRQCHIP", "IOAPIC") => "KVM_GET_IRQCHIP IOAPIC",
        ("KVM_SET_IRQCHIP", "IOAPIC") => "KVM_SET_IRQCHIP IOAPIC",
        ("validate KVM_GET_IRQCHIP", "IOAPIC") => "validate KVM_GET_IRQCHIP IOAPIC",
        _ => "KVM irqchip checkpoint operation",
    };
    crate::error::Error::HostEnvironment(crate::error::HostEnvironmentError::VmOperation {
        operation,
        source,
    })
}

fn get_irqchip(fd: std::os::fd::RawFd, request: &mut KvmIrqchip) -> std::io::Result<()> {
    // SAFETY: `request` is the exact fixed 520-byte Linux `struct kvm_irqchip` payload and remains
    // writable for the duration of the VM ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_GET_IRQCHIP, request) };
    cvt_ioctl(result).map(|_| ())
}

fn set_irqchip(fd: std::os::fd::RawFd, request: &KvmIrqchip) -> std::io::Result<()> {
    // SAFETY: `request` is the exact fixed 520-byte Linux `struct kvm_irqchip` payload and remains
    // readable for the duration of the VM ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_SET_IRQCHIP, request) };
    cvt_ioctl(result).map(|_| ())
}

fn decode_pic_state(bytes: &[u8; KVM_IRQCHIP_PAYLOAD_SIZE]) -> KvmPicState {
    KvmPicState {
        last_irr: bytes[0],
        irr: bytes[1],
        imr: bytes[2],
        isr: bytes[3],
        priority_add: bytes[4],
        irq_base: bytes[5],
        read_reg_select: bytes[6],
        poll: bytes[7],
        special_mask: bytes[8],
        init_state: bytes[9],
        auto_eoi: bytes[10],
        rotate_on_auto_eoi: bytes[11],
        special_fully_nested_mode: bytes[12],
        init4: bytes[13],
        elcr: bytes[14],
        elcr_mask: bytes[15],
    }
}

fn encode_pic_state(state: KvmPicState, bytes: &mut [u8; KVM_IRQCHIP_PAYLOAD_SIZE]) {
    bytes[..16].copy_from_slice(&[
        state.last_irr,
        state.irr,
        state.imr,
        state.isr,
        state.priority_add,
        state.irq_base,
        state.read_reg_select,
        state.poll,
        state.special_mask,
        state.init_state,
        state.auto_eoi,
        state.rotate_on_auto_eoi,
        state.special_fully_nested_mode,
        state.init4,
        state.elcr,
        state.elcr_mask,
    ]);
}

fn decode_ioapic_state(bytes: &[u8; KVM_IRQCHIP_PAYLOAD_SIZE]) -> KvmIoapicState {
    let mut redirtbl = [0u64; KVM_IOAPIC_NUM_PINS];
    for (index, entry) in redirtbl.iter_mut().enumerate() {
        let offset = 24 + index * 8;
        *entry = u64::from_le_bytes(
            bytes[offset..offset + 8]
                .try_into()
                .expect("fixed IOAPIC redirection entry remains in the 512-byte payload"),
        );
    }
    KvmIoapicState {
        base_address: u64::from_le_bytes(bytes[0..8].try_into().expect("IOAPIC base fits")),
        ioregsel: u32::from_le_bytes(bytes[8..12].try_into().expect("IOAPIC ioregsel fits")),
        id: u32::from_le_bytes(bytes[12..16].try_into().expect("IOAPIC id fits")),
        irr: u32::from_le_bytes(bytes[16..20].try_into().expect("IOAPIC irr fits")),
        pad: u32::from_le_bytes(bytes[20..24].try_into().expect("IOAPIC pad fits")),
        redirtbl,
    }
}

fn encode_ioapic_state(state: KvmIoapicState, bytes: &mut [u8; KVM_IRQCHIP_PAYLOAD_SIZE]) {
    bytes[..KVM_IOAPIC_STATE_SIZE].fill(0);
    bytes[0..8].copy_from_slice(&state.base_address.to_le_bytes());
    bytes[8..12].copy_from_slice(&state.ioregsel.to_le_bytes());
    bytes[12..16].copy_from_slice(&state.id.to_le_bytes());
    bytes[16..20].copy_from_slice(&state.irr.to_le_bytes());
    bytes[20..24].copy_from_slice(&state.pad.to_le_bytes());
    for (index, entry) in state.redirtbl.iter().enumerate() {
        let offset = 24 + index * 8;
        bytes[offset..offset + 8].copy_from_slice(&entry.to_le_bytes());
    }
}

const _: () = {
    assert!(std::mem::size_of::<KvmPicState>() == 16);
    assert!(std::mem::size_of::<KvmIoapicState>() == KVM_IOAPIC_STATE_SIZE);
    assert!(std::mem::size_of::<KvmIrqchip>() == 520);
};

#[cfg(test)]
mod irqchip_snapshot_uapi_tests {
    use super::*;

    #[test]
    fn irqchip_snapshot_uapi_matches_linux_x86_kvm() {
        assert_eq!(KVM_GET_IRQCHIP, 0xC208_AE62);
        assert_eq!(KVM_SET_IRQCHIP, 0x8208_AE63);
        assert_eq!(KVM_IRQCHIP_PIC_MASTER, 0);
        assert_eq!(KVM_IRQCHIP_PIC_SLAVE, 1);
        assert_eq!(KVM_IRQCHIP_IOAPIC, 2);
        assert_eq!(KVM_IOAPIC_NUM_PINS, 24);
        assert_eq!(std::mem::size_of::<KvmPicState>(), 16);
        assert_eq!(std::mem::size_of::<KvmIoapicState>(), 216);
        assert_eq!(std::mem::size_of::<KvmIrqchip>(), 520);
    }

    #[test]
    fn master_pic_snapshot_round_trips_exact_sixteen_byte_state() {
        let mut request = KvmIrqchip::request(KVM_IRQCHIP_PIC_MASTER);
        for (index, byte) in request.chip[..16].iter_mut().enumerate() {
            *byte = index as u8;
        }
        request.chip[16] = 0xee;
        let snapshot = MasterPicStateSnapshot {
            state: decode_pic_state(&request.chip),
        };
        assert_eq!(snapshot.imr(), 2);

        let mutated = snapshot.with_imr(0xfe);
        let mut encoded = KvmIrqchip::request(KVM_IRQCHIP_PIC_MASTER);
        encode_pic_state(mutated.state, &mut encoded.chip);
        assert_eq!(encoded.chip[0], 0);
        assert_eq!(encoded.chip[1], 1);
        assert_eq!(encoded.chip[2], 0xfe);
        assert_eq!(encoded.chip[3..16], request.chip[3..16]);
        assert_eq!(encoded.chip[16..], [0; KVM_IRQCHIP_PAYLOAD_SIZE - 16]);
    }

    #[test]
    fn slave_pic_snapshot_uses_the_same_exact_sixteen_byte_state() {
        let mut request = KvmIrqchip::request(KVM_IRQCHIP_PIC_SLAVE);
        for (index, byte) in request.chip[..16].iter_mut().enumerate() {
            *byte = (0x80 + index) as u8;
        }
        let snapshot = SlavePicStateSnapshot {
            state: decode_pic_state(&request.chip),
        };
        assert_eq!(snapshot.imr(), 0x82);
        let mutated = snapshot.with_imr(0xfe);
        let mut encoded = KvmIrqchip::request(KVM_IRQCHIP_PIC_SLAVE);
        encode_pic_state(mutated.state, &mut encoded.chip);
        assert_eq!(encoded.chip[2], 0xfe);
        assert_eq!(encoded.chip[0..2], request.chip[0..2]);
        assert_eq!(encoded.chip[3..16], request.chip[3..16]);
    }

    #[test]
    fn ioapic_snapshot_round_trips_only_the_216_byte_semantic_state() {
        let state = KvmIoapicState {
            base_address: 0xfec0_0000,
            ioregsel: 0x12,
            id: 0x0f00_0000,
            irr: 0,
            pad: 0,
            redirtbl: std::array::from_fn(|index| 0x0001_0000_0000_0040 + index as u64),
        };
        let mut payload = [0xa5; KVM_IRQCHIP_PAYLOAD_SIZE];
        encode_ioapic_state(state, &mut payload);
        assert_eq!(decode_ioapic_state(&payload), state);
        assert_eq!(payload[KVM_IOAPIC_STATE_SIZE..], [0xa5; KVM_IRQCHIP_PAYLOAD_SIZE - KVM_IOAPIC_STATE_SIZE]);

        let snapshot = IoapicStateSnapshot { state };
        assert_eq!(snapshot.base_address(), 0xfec0_0000);
        assert_eq!(snapshot.ioregsel(), 0x12);
        assert_eq!(snapshot.id(), 0x0f00_0000);
        assert_eq!(snapshot.irr(), 0);
        assert_eq!(snapshot.pad(), 0);
        assert_eq!(snapshot.redirection_entry(16), Some(state.redirtbl[16]));
        assert_eq!(snapshot.redirection_entry(KVM_IOAPIC_NUM_PINS), None);

        let changed = snapshot.with_redirection_entry(16, 0x50).unwrap();
        assert_eq!(changed.redirection_entry(16), Some(0x50));
        assert_eq!(snapshot.redirection_entry(16), Some(state.redirtbl[16]));
        assert!(snapshot.with_redirection_entry(KVM_IOAPIC_NUM_PINS, 0).is_none());
    }
}
