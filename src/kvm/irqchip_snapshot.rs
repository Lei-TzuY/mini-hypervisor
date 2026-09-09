pub const KVM_GET_IRQCHIP: libc::c_ulong = 0xC208_AE62;
pub const KVM_SET_IRQCHIP: libc::c_ulong = 0x8208_AE63;
const KVM_IRQCHIP_PIC_MASTER: u32 = 0;
const KVM_IRQCHIP_PAYLOAD_SIZE: usize = 512;

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
#[derive(Debug, Clone, PartialEq, Eq)]
struct KvmIrqchip {
    chip_id: u32,
    pad: u32,
    chip: [u8; KVM_IRQCHIP_PAYLOAD_SIZE],
}

impl KvmIrqchip {
    fn master_pic_request() -> Self {
        Self {
            chip_id: KVM_IRQCHIP_PIC_MASTER,
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

impl crate::kvm::Vm {
    pub(crate) fn capture_master_pic_state(
        &self,
    ) -> Result<MasterPicStateSnapshot, crate::error::Error> {
        let fd = std::os::fd::AsRawFd::as_raw_fd(&self.fd);
        let mut request = KvmIrqchip::master_pic_request();
        get_irqchip(fd, &mut request).map_err(|source| {
            crate::error::Error::HostEnvironment(crate::error::HostEnvironmentError::VmOperation {
                operation: "KVM_GET_IRQCHIP master PIC",
                source,
            })
        })?;
        if request.chip_id != KVM_IRQCHIP_PIC_MASTER || request.pad != 0 {
            return Err(crate::error::Error::HostEnvironment(
                crate::error::HostEnvironmentError::VmOperation {
                    operation: "validate KVM_GET_IRQCHIP master PIC",
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "expected master PIC chip_id 0 and zero pad, got chip_id {} pad {}",
                            request.chip_id, request.pad
                        ),
                    ),
                },
            ));
        }
        Ok(MasterPicStateSnapshot {
            state: decode_pic_state(&request.chip),
        })
    }

    pub(crate) fn restore_master_pic_state(
        &self,
        snapshot: &MasterPicStateSnapshot,
    ) -> Result<(), crate::error::Error> {
        let fd = std::os::fd::AsRawFd::as_raw_fd(&self.fd);
        let mut request = KvmIrqchip::master_pic_request();
        encode_pic_state(snapshot.state, &mut request.chip);
        set_irqchip(fd, &request).map_err(|source| {
            crate::error::Error::HostEnvironment(crate::error::HostEnvironmentError::VmOperation {
                operation: "KVM_SET_IRQCHIP master PIC",
                source,
            })
        })
    }
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

const _: () = {
    assert!(std::mem::size_of::<KvmPicState>() == 16);
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
        assert_eq!(std::mem::size_of::<KvmPicState>(), 16);
        assert_eq!(std::mem::size_of::<KvmIrqchip>(), 520);
    }

    #[test]
    fn master_pic_snapshot_round_trips_exact_sixteen_byte_state() {
        let mut request = KvmIrqchip::master_pic_request();
        for (index, byte) in request.chip[..16].iter_mut().enumerate() {
            *byte = index as u8;
        }
        request.chip[16] = 0xee;
        let snapshot = MasterPicStateSnapshot {
            state: decode_pic_state(&request.chip),
        };
        assert_eq!(snapshot.imr(), 2);

        let mutated = snapshot.with_imr(0xfe);
        let mut encoded = KvmIrqchip::master_pic_request();
        encode_pic_state(mutated.state, &mut encoded.chip);
        assert_eq!(encoded.chip[0], 0);
        assert_eq!(encoded.chip[1], 1);
        assert_eq!(encoded.chip[2], 0xfe);
        assert_eq!(encoded.chip[3..16], request.chip[3..16]);
        assert_eq!(encoded.chip[16..], [0; KVM_IRQCHIP_PAYLOAD_SIZE - 16]);
    }
}
