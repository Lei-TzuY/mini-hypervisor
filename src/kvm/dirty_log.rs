const KVM_GET_DIRTY_LOG: libc::c_ulong = 0x4010_AE42;
const KVM_MEM_LOG_DIRTY_PAGES: u32 = 1 << 0;
const DIRTY_LOG_SLOT: u32 = 0;
const DIRTY_LOG_RAM_PAGES: u64 = 4;
const DIRTY_LOG_RAM_SIZE: u64 = DIRTY_LOG_RAM_PAGES * crate::memory::KVM_MEMORY_ALIGNMENT;
const DIRTY_LOG_GUEST_ENTRY: crate::memory::GuestPhysAddr = crate::memory::GuestPhysAddr::new(0x1100);
const DIRTY_LOG_FIRST_WRITE: crate::memory::GuestPhysAddr = crate::memory::GuestPhysAddr::new(0x1000);
const DIRTY_LOG_SECOND_WRITE: crate::memory::GuestPhysAddr = crate::memory::GuestPhysAddr::new(0x3000);
const DIRTY_LOG_FIRST_VALUE: u8 = b'A';
const DIRTY_LOG_SECOND_VALUE: u8 = b'B';
const DIRTY_LOG_PROOF: &[u8; 2] = b"DG";
const DIRTY_LOG_TERMINAL_RIP: u64 = 0x1113;
const DIRTY_LOG_EXIT_BUDGET: u32 = 3;
const DIRTY_LOG_EXPECTED_BITMAP: u64 = (1 << 1) | (1 << 3);

const DIRTY_LOG_GUEST_BYTES: [u8; 19] = [
    0xc6, 0x06, 0x00, 0x10, DIRTY_LOG_FIRST_VALUE, // mov byte [0x1000], 'A'
    0xc6, 0x06, 0x00, 0x30, DIRTY_LOG_SECOND_VALUE, // mov byte [0x3000], 'B'
    0xb0, b'D', 0xe6, 0xe9, // out 0xe9, 'D'
    0xb0, b'G', 0xe6, 0xe9, // out 0xe9, 'G'
    0xf4, // hlt
];

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmDirtyLog {
    slot: u32,
    padding: u32,
    dirty_bitmap: u64,
}

impl KvmDirtyLog {
    fn slot0(bitmap: &mut [u64]) -> Self {
        debug_assert!(!bitmap.is_empty());
        Self {
            slot: DIRTY_LOG_SLOT,
            padding: 0,
            dirty_bitmap: bitmap.as_mut_ptr() as usize as u64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirtyLogSlot0 {
    page_count: u64,
    bitmap_words: usize,
}

impl DirtyLogSlot0 {
    fn for_region(region: crate::memory::GuestMemoryRegion) -> Self {
        debug_assert_eq!(region.size() % crate::memory::KVM_MEMORY_ALIGNMENT, 0);
        let page_count = region.size() / crate::memory::KVM_MEMORY_ALIGNMENT;
        let bitmap_words = usize::try_from(page_count.div_ceil(64))
            .expect("validated guest-memory page count fits host usize");
        Self {
            page_count,
            bitmap_words,
        }
    }
}

type DirtyLogGuestRun = (
    Vec<u64>,
    Vec<u64>,
    [u8; 2],
    Vec<u8>,
    crate::vmexit::VmExitReport,
);

impl crate::kvm::KvmBackend {
    pub const DIRTY_LOG_PROOF: &'static [u8; 2] = DIRTY_LOG_PROOF;
    pub const DIRTY_LOG_EXPECTED_BITMAP: u64 = DIRTY_LOG_EXPECTED_BITMAP;
    pub const DIRTY_LOG_TERMINAL_RIP: u64 = DIRTY_LOG_TERMINAL_RIP;

    pub fn run_dirty_log_guest(
        config: crate::config::VmConfig,
    ) -> Result<DirtyLogGuestRun, crate::error::Error> {
        let image = crate::loader::FlatGuestImage::new(
            DIRTY_LOG_GUEST_ENTRY,
            DIRTY_LOG_GUEST_ENTRY,
            &DIRTY_LOG_GUEST_BYTES,
        )?;
        let backend = Self::open()?;
        let mut vm = backend.create_vm()?;
        let mut memory = crate::memory::GuestMemory::new(
            crate::memory::GuestPhysAddr::new(0),
            DIRTY_LOG_RAM_SIZE,
        )?;

        // Finish every host-side guest-image and vCPU state initialization step before enabling
        // dirty logging. KVM may still report implementation/setup dirtiness when a logging memslot
        // is installed, so explicitly drain that state and prove a second pre-run harvest is clean.
        // The measured guest interval begins only after this verified zero baseline and immediately
        // before the first KVM_RUN. Keep executable bytes on page 1, which the guest explicitly
        // dirties at 0x1000, so conservative execution-page dirtiness cannot create a third tracked
        // page and obscure the exact two-page fixture contract.
        image.load(&mut memory)?;
        debug_assert_eq!(config.vcpu_count(), 1);
        let mut vcpu = vm.create_vcpu(crate::vcpu::VcpuId::BOOT)?;
        vcpu.initialize_real_mode(image.entry())?;
        let dirty_slot = register_guest_memory_with_dirty_log(&mut vm, memory)?;

        let setup_dirty = harvest_dirty_log(&vm, dirty_slot)?;
        let clean_baseline = harvest_dirty_log(&vm, dirty_slot)?;
        if clean_baseline.iter().any(|word| *word != 0) {
            return Err(dirty_log_verification_error(
                "pre-execution dirty-log baseline",
                format!(
                    "expected a clean bitmap after draining setup dirtiness {setup_dirty:?}, got {clean_baseline:?}"
                ),
            ));
        }

        let mut port_io = crate::portio::PortIoBus::with_debug_port();
        let execution = crate::execution::run_vcpu_until_stopped(
            &mut vcpu,
            &mut port_io,
            DIRTY_LOG_EXIT_BUDGET,
        )?;

        let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
        validate_dirty_log_execution(&execution, &proof)?;

        let first = harvest_dirty_log(&vm, dirty_slot)?;
        if first.as_slice() != [DIRTY_LOG_EXPECTED_BITMAP] {
            return Err(dirty_log_verification_error(
                "first dirty-log harvest",
                format!(
                    "expected exact slot-0 bitmap [{DIRTY_LOG_EXPECTED_BITMAP:#x}], got {first:?}"
                ),
            ));
        }

        let guest_memory = vm
            .guest_memory()
            .expect("dirty-log registration retains VM-owned guest memory");
        let mut values = [0_u8; 2];
        guest_memory.read(DIRTY_LOG_FIRST_WRITE, &mut values[..1])?;
        guest_memory.read(DIRTY_LOG_SECOND_WRITE, &mut values[1..])?;
        if values != [DIRTY_LOG_FIRST_VALUE, DIRTY_LOG_SECOND_VALUE] {
            return Err(dirty_log_verification_error(
                "dirty guest memory contents",
                format!("expected [65, 66], got {values:?}"),
            ));
        }

        // Basic KVM_GET_DIRTY_LOG clears harvested bits before returning when manual dirty-log
        // protect2 is not enabled. No guest is re-entered between these two post-run harvests.
        let second = harvest_dirty_log(&vm, dirty_slot)?;
        if second.iter().any(|word| *word != 0) {
            return Err(dirty_log_verification_error(
                "second dirty-log harvest",
                format!("expected a fully cleared bitmap without new guest writes, got {second:?}"),
            ));
        }

        Ok((first, second, values, proof, execution.report()))
    }
}

pub(crate) fn register_guest_memory_with_dirty_log(
    vm: &mut crate::kvm::Vm,
    memory: crate::memory::GuestMemory,
) -> Result<DirtyLogSlot0, crate::error::Error> {
    if vm.guest_memory.is_some() {
        return Err(crate::error::Error::GuestMemory(
            crate::error::GuestMemoryError::AlreadyRegistered,
        ));
    }

    super::validate_guest_memory_registration(memory.region())?;
    let region = memory.region();
    let request = KvmUserspaceMemoryRegion {
        slot: DIRTY_LOG_SLOT,
        flags: KVM_MEM_LOG_DIRTY_PAGES,
        guest_phys_addr: region.base().get(),
        memory_size: region.size(),
        userspace_addr: memory.userspace_addr(),
    };
    set_user_memory_region(std::os::fd::AsRawFd::as_raw_fd(&vm.fd), &request)
        .map_err(|source| {
            crate::error::Error::GuestMemory(crate::error::GuestMemoryError::Registration { source })
        })?;
    vm.guest_memory = Some(memory);
    Ok(DirtyLogSlot0::for_region(region))
}

pub(crate) fn harvest_dirty_log(
    vm: &crate::kvm::Vm,
    slot: DirtyLogSlot0,
) -> Result<Vec<u64>, crate::error::Error> {
    if vm.guest_memory.is_none() {
        return Err(dirty_log_vm_error(
            "KVM_GET_DIRTY_LOG",
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "dirty-log harvest requires registered guest memory",
            ),
        ));
    }
    let mut bitmap = vec![0_u64; slot.bitmap_words];
    let request = KvmDirtyLog::slot0(&mut bitmap);
    get_dirty_log(std::os::fd::AsRawFd::as_raw_fd(&vm.fd), &request)
        .map_err(|source| dirty_log_vm_error("KVM_GET_DIRTY_LOG", source))?;
    mask_unused_bitmap_bits(&mut bitmap, slot.page_count);
    Ok(bitmap)
}

fn get_dirty_log(fd: std::os::fd::RawFd, request: &KvmDirtyLog) -> std::io::Result<()> {
    // SAFETY: `request` is the exact x86_64 KVM UAPI layout. Its bitmap address points to a
    // writable Vec<u64> that remains alive for the whole ioctl, and padding is explicitly zero.
    let result = unsafe { libc::ioctl(fd, KVM_GET_DIRTY_LOG, request) };
    cvt_ioctl(result).map(|_| ())
}

fn mask_unused_bitmap_bits(bitmap: &mut [u64], page_count: u64) {
    let used_in_last = page_count % 64;
    if used_in_last == 0 {
        return;
    }
    if let Some(last) = bitmap.last_mut() {
        *last &= (1_u64 << used_in_last) - 1;
    }
}

fn validate_dirty_log_execution(
    execution: &crate::execution::VmExecutionResult,
    proof: &[u8],
) -> Result<(), crate::error::Error> {
    if execution.completed_exits() != DIRTY_LOG_EXIT_BUDGET
        || execution.io_exits().len() != DIRTY_LOG_PROOF.len()
        || proof != DIRTY_LOG_PROOF
    {
        return Err(dirty_log_verification_error(
            "dirty-log debug proof",
            format!(
                "expected {:?} across {} I/O exits and {} total exits, got {:?} across {} I/O exits and {} total exits",
                DIRTY_LOG_PROOF,
                DIRTY_LOG_PROOF.len(),
                DIRTY_LOG_EXIT_BUDGET,
                proof,
                execution.io_exits().len(),
                execution.completed_exits()
            ),
        ));
    }
    for (exit, expected) in execution
        .io_exits()
        .iter()
        .zip(DIRTY_LOG_PROOF.iter().copied())
    {
        if exit.direction() != crate::vcpu::PortIoDirection::Out
            || exit.port() != crate::portio::DEBUG_PORT
            || exit.size() != 1
            || exit.count() != 1
            || exit.output_data() != [expected]
        {
            return Err(dirty_log_verification_error(
                "dirty-log debug-port exit",
                format!("unexpected I/O exit {exit:?}, expected byte {expected:#x}"),
            ));
        }
    }

    let report = execution.report();
    if report.exit() != crate::vcpu::VcpuExit::Hlt
        || report.rip() != DIRTY_LOG_TERMINAL_RIP
        || report.rflags() & 0x2 != 0x2
    {
        return Err(dirty_log_verification_error(
            "dirty-log terminal exit",
            format!(
                "expected HLT at {DIRTY_LOG_TERMINAL_RIP:#x} with RFLAGS bit1 set, got {report}"
            ),
        ));
    }
    Ok(())
}

fn dirty_log_vm_error(operation: &'static str, source: std::io::Error) -> crate::error::Error {
    crate::error::Error::HostEnvironment(crate::error::HostEnvironmentError::VmOperation {
        operation,
        source,
    })
}

fn dirty_log_verification_error(
    operation: &'static str,
    detail: impl Into<String>,
) -> crate::error::Error {
    dirty_log_vm_error(
        operation,
        std::io::Error::new(std::io::ErrorKind::InvalidData, detail.into()),
    )
}

#[cfg(test)]
mod dirty_log_tests {
    use super::*;

    #[test]
    fn dirty_log_uapi_matches_linux_x86_64() {
        assert_eq!(KVM_GET_DIRTY_LOG, 0x4010_AE42);
        assert_eq!(KVM_MEM_LOG_DIRTY_PAGES, 1);
        assert_eq!(std::mem::size_of::<KvmDirtyLog>(), 16);
        assert_eq!(std::mem::offset_of!(KvmDirtyLog, dirty_bitmap), 8);
        let mut bitmap = [0_u64; 1];
        let request = KvmDirtyLog::slot0(&mut bitmap);
        assert_eq!(request.slot, 0);
        assert_eq!(request.padding, 0);
        assert_ne!(request.dirty_bitmap, 0);
    }

    #[test]
    fn four_page_slot_uses_one_word_and_exact_page_bits() {
        let region = crate::memory::GuestMemoryRegion::new(
            crate::memory::GuestPhysAddr::new(0),
            DIRTY_LOG_RAM_SIZE,
        )
        .unwrap();
        let slot = DirtyLogSlot0::for_region(region);
        assert_eq!(slot.page_count, 4);
        assert_eq!(slot.bitmap_words, 1);
        assert_eq!(DIRTY_LOG_EXPECTED_BITMAP, 0b1010);
    }

    #[test]
    fn unused_bitmap_bits_are_masked_without_touching_valid_pages() {
        let mut words = [u64::MAX];
        mask_unused_bitmap_bits(&mut words, 4);
        assert_eq!(words, [0xf]);
    }

    #[test]
    fn real_mode_fixture_executes_from_page_one_and_only_explicitly_writes_pages_one_and_three() {
        assert_eq!(DIRTY_LOG_GUEST_BYTES.len(), 19);
        assert_eq!(
            DIRTY_LOG_GUEST_ENTRY.get() / crate::memory::KVM_MEMORY_ALIGNMENT,
            1
        );
        assert_eq!(
            DIRTY_LOG_FIRST_WRITE.get() / crate::memory::KVM_MEMORY_ALIGNMENT,
            1
        );
        assert_eq!(
            DIRTY_LOG_SECOND_WRITE.get() / crate::memory::KVM_MEMORY_ALIGNMENT,
            3
        );
        assert_eq!(
            DIRTY_LOG_GUEST_ENTRY.get() + DIRTY_LOG_GUEST_BYTES.len() as u64,
            DIRTY_LOG_TERMINAL_RIP
        );
        assert_eq!(&DIRTY_LOG_GUEST_BYTES[..5], &[0xc6, 0x06, 0x00, 0x10, b'A']);
        assert_eq!(&DIRTY_LOG_GUEST_BYTES[5..10], &[0xc6, 0x06, 0x00, 0x30, b'B']);
        assert_eq!(&DIRTY_LOG_GUEST_BYTES[10..18], &[0xb0, b'D', 0xe6, 0xe9, 0xb0, b'G', 0xe6, 0xe9]);
        assert_eq!(DIRTY_LOG_GUEST_BYTES[18], 0xf4);
        assert_eq!(DIRTY_LOG_PROOF, b"DG");
    }
}