fn encoded_length(
    page_count: usize,
    msr_count: usize,
) -> Result<usize, VersionedPageVcpuCheckpointError> {
    let pages = PAGE_RECORD_LEN
        .checked_mul(page_count)
        .ok_or(VersionedPageVcpuCheckpointError::LengthOverflow)?;
    let msrs = MSR_RECORD_LEN
        .checked_mul(msr_count)
        .ok_or(VersionedPageVcpuCheckpointError::LengthOverflow)?;
    HEADER_LEN
        .checked_add(pages)
        .and_then(|length| length.checked_add(REGISTER_BLOCK_LEN))
        .and_then(|length| length.checked_add(SPECIAL_REGISTER_BLOCK_LEN))
        .and_then(|length| length.checked_add(msrs))
        .ok_or(VersionedPageVcpuCheckpointError::LengthOverflow)
}

fn validate_pages(pages: &[BoundedCheckpointPage]) -> Result<(), VersionedPageVcpuCheckpointError> {
    if pages.is_empty() || pages.len() > BOUNDED_CHECKPOINT_PAGE_SET_LIMIT {
        return Err(VersionedPageVcpuCheckpointError::InvalidPageCount(
            u16::try_from(pages.len()).unwrap_or(u16::MAX),
        ));
    }
    let mut previous = None;
    for page in pages {
        let address = page.address().get();
        validate_page_address(address)?;
        if page.bytes().len() != LONG_MODE_PAGE_SIZE as usize {
            return Err(VersionedPageVcpuCheckpointError::InvalidPageLength {
                address,
                length: page.bytes().len(),
            });
        }
        if let Some(previous) = previous {
            if address <= previous {
                return Err(VersionedPageVcpuCheckpointError::NonCanonicalPageOrder {
                    previous,
                    current: address,
                });
            }
        }
        previous = Some(address);
    }
    Ok(())
}

fn validate_page_address(address: u64) -> Result<(), VersionedPageVcpuCheckpointError> {
    if address % LONG_MODE_PAGE_SIZE != 0 {
        return Err(VersionedPageVcpuCheckpointError::UnalignedPage(address));
    }
    address.checked_add(LONG_MODE_PAGE_SIZE).ok_or(
        VersionedPageVcpuCheckpointError::PageAddressOverflow(address),
    )?;
    Ok(())
}

fn validate_msr_order(msrs: &[(MsrIndex, u64)]) -> Result<(), VersionedPageVcpuCheckpointError> {
    let mut previous = None;
    for (index, _) in msrs {
        let current = index.get();
        if let Some(previous) = previous {
            if current <= previous {
                return Err(VersionedPageVcpuCheckpointError::NonCanonicalMsrOrder {
                    previous,
                    current,
                });
            }
        }
        previous = Some(current);
    }
    Ok(())
}

fn encode_registers(bytes: &mut Vec<u8>, registers: &VcpuRegisterSnapshot) {
    for value in [
        registers.rax(),
        registers.rbx(),
        registers.rcx(),
        registers.rdx(),
        registers.rsi(),
        registers.rdi(),
        registers.rsp(),
        registers.rbp(),
        registers.r8(),
        registers.r9(),
        registers.r10(),
        registers.r11(),
        registers.r12(),
        registers.r13(),
        registers.r14(),
        registers.r15(),
        registers.rip(),
        registers.rflags(),
    ] {
        push_u64(bytes, value);
    }
}

fn decode_registers(
    cursor: &mut Cursor<'_>,
) -> Result<sys::KvmRegs, VersionedPageVcpuCheckpointError> {
    Ok(sys::KvmRegs {
        rax: cursor.u64()?,
        rbx: cursor.u64()?,
        rcx: cursor.u64()?,
        rdx: cursor.u64()?,
        rsi: cursor.u64()?,
        rdi: cursor.u64()?,
        rsp: cursor.u64()?,
        rbp: cursor.u64()?,
        r8: cursor.u64()?,
        r9: cursor.u64()?,
        r10: cursor.u64()?,
        r11: cursor.u64()?,
        r12: cursor.u64()?,
        r13: cursor.u64()?,
        r14: cursor.u64()?,
        r15: cursor.u64()?,
        rip: cursor.u64()?,
        rflags: cursor.u64()?,
    })
}

fn encode_special_registers(bytes: &mut Vec<u8>, sregs: &VcpuSpecialRegisterSnapshot) {
    for segment in [
        sregs.cs(),
        sregs.ds(),
        sregs.es(),
        sregs.fs(),
        sregs.gs(),
        sregs.ss(),
        sregs.tr(),
        sregs.ldt(),
    ] {
        encode_segment(bytes, segment);
    }
    encode_dtable(bytes, sregs.gdt());
    encode_dtable(bytes, sregs.idt());
    for value in [
        sregs.cr0(),
        sregs.cr2(),
        sregs.cr3(),
        sregs.cr4(),
        sregs.cr8(),
        sregs.efer(),
        sregs.apic_base(),
        sregs.interrupt_bitmap()[0],
        sregs.interrupt_bitmap()[1],
        sregs.interrupt_bitmap()[2],
        sregs.interrupt_bitmap()[3],
    ] {
        push_u64(bytes, value);
    }
}

fn decode_special_registers(
    cursor: &mut Cursor<'_>,
) -> Result<sys::KvmSregs, VersionedPageVcpuCheckpointError> {
    let mut segments = [sys::KvmSegment::default(); 8];
    for (index, segment) in segments.iter_mut().enumerate() {
        *segment = decode_segment(cursor, index as u8)?;
    }
    let gdt = decode_dtable(cursor, "gdt.reserved")?;
    let idt = decode_dtable(cursor, "idt.reserved")?;
    Ok(sys::KvmSregs {
        cs: segments[0],
        ds: segments[1],
        es: segments[2],
        fs: segments[3],
        gs: segments[4],
        ss: segments[5],
        tr: segments[6],
        ldt: segments[7],
        gdt,
        idt,
        cr0: cursor.u64()?,
        cr2: cursor.u64()?,
        cr3: cursor.u64()?,
        cr4: cursor.u64()?,
        cr8: cursor.u64()?,
        efer: cursor.u64()?,
        apic_base: cursor.u64()?,
        interrupt_bitmap: [cursor.u64()?, cursor.u64()?, cursor.u64()?, cursor.u64()?],
    })
}

fn encode_segment(bytes: &mut Vec<u8>, segment: VcpuSegmentState) {
    push_u64(bytes, segment.base());
    push_u32(bytes, segment.limit());
    push_u16(bytes, segment.selector());
    bytes.extend_from_slice(&[
        segment.segment_type(),
        segment.present(),
        segment.dpl(),
        segment.db(),
        segment.s(),
        segment.l(),
        segment.g(),
        segment.avl(),
        segment.unusable(),
        0,
    ]);
}

fn decode_segment(
    cursor: &mut Cursor<'_>,
    segment: u8,
) -> Result<sys::KvmSegment, VersionedPageVcpuCheckpointError> {
    let base = cursor.u64()?;
    let limit = cursor.u32()?;
    let selector = cursor.u16()?;
    let type_ = cursor.u8()?;
    let present = cursor.u8()?;
    let dpl = cursor.u8()?;
    let db = cursor.u8()?;
    let s = cursor.u8()?;
    let l = cursor.u8()?;
    let g = cursor.u8()?;
    let avl = cursor.u8()?;
    let unusable = cursor.u8()?;
    let reserved = cursor.u8()?;
    if type_ > 0x0f {
        return Err(VersionedPageVcpuCheckpointError::InvalidSegmentField {
            segment,
            field: "type",
            value: type_,
        });
    }
    if dpl > 3 {
        return Err(VersionedPageVcpuCheckpointError::InvalidSegmentField {
            segment,
            field: "dpl",
            value: dpl,
        });
    }
    for (field, value) in [
        ("present", present),
        ("db", db),
        ("s", s),
        ("l", l),
        ("g", g),
        ("avl", avl),
        ("unusable", unusable),
    ] {
        if value > 1 {
            return Err(VersionedPageVcpuCheckpointError::InvalidSegmentField {
                segment,
                field,
                value,
            });
        }
    }
    if reserved != 0 {
        return Err(VersionedPageVcpuCheckpointError::NonZeroReserved {
            field: "segment.reserved",
            value: u64::from(reserved),
        });
    }
    Ok(sys::KvmSegment {
        base,
        limit,
        selector,
        type_,
        present,
        dpl,
        db,
        s,
        l,
        g,
        avl,
        unusable,
        padding: 0,
    })
}

fn encode_dtable(bytes: &mut Vec<u8>, table: VcpuDescriptorTableState) {
    push_u64(bytes, table.base());
    push_u16(bytes, table.limit());
    bytes.extend_from_slice(&[0; 6]);
}

fn decode_dtable(
    cursor: &mut Cursor<'_>,
    reserved_field: &'static str,
) -> Result<sys::KvmDtable, VersionedPageVcpuCheckpointError> {
    let base = cursor.u64()?;
    let limit = cursor.u16()?;
    let reserved = cursor.take(6)?;
    if reserved.iter().any(|byte| *byte != 0) {
        let value = reserved
            .iter()
            .enumerate()
            .fold(0_u64, |acc, (index, byte)| {
                acc | (u64::from(*byte) << (8 * index))
            });
        return Err(VersionedPageVcpuCheckpointError::NonZeroReserved {
            field: reserved_field,
            value,
        });
    }
    Ok(sys::KvmDtable {
        base,
        limit,
        padding: [0; 3],
    })
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

