struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], VersionedPageVcpuCheckpointError> {
        let remaining = self.remaining();
        if length > remaining {
            return Err(VersionedPageVcpuCheckpointError::Truncated {
                offset: self.offset,
                needed: length,
                remaining,
            });
        }
        let start = self.offset;
        self.offset += length;
        Ok(&self.bytes[start..self.offset])
    }

    fn u8(&mut self) -> Result<u8, VersionedPageVcpuCheckpointError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, VersionedPageVcpuCheckpointError> {
        let bytes: [u8; 2] = self.take(2)?.try_into().expect("cursor returned two bytes");
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, VersionedPageVcpuCheckpointError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .expect("cursor returned four bytes");
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, VersionedPageVcpuCheckpointError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .expect("cursor returned eight bytes");
        Ok(u64::from_le_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(seed: u8) -> sys::KvmSegment {
        sys::KvmSegment {
            base: u64::from(seed) << 32,
            limit: 0xffff,
            selector: u16::from(seed) << 3,
            type_: seed & 0x0f,
            present: 1,
            dpl: seed & 0x03,
            db: 0,
            s: 1,
            l: 1,
            g: 1,
            avl: 0,
            unusable: 0,
            padding: 0,
        }
    }

    fn checkpoint() -> (BoundedVcpuPageSetCheckpoint, HostMsrIndexList) {
        let registers = VcpuRegisterSnapshot::from_kvm_regs(sys::KvmRegs {
            rax: 1,
            rbx: 2,
            rcx: 3,
            rdx: 4,
            rsi: 5,
            rdi: 6,
            rsp: 7,
            rbp: 8,
            r8: 9,
            r9: 10,
            r10: 11,
            r11: 12,
            r12: 13,
            r13: 14,
            r14: 15,
            r15: 16,
            rip: 17,
            rflags: 0x202,
        });
        let special_registers = VcpuSpecialRegisterSnapshot::from_kvm_sregs(sys::KvmSregs {
            cs: segment(1),
            ds: segment(2),
            es: segment(3),
            fs: segment(4),
            gs: segment(5),
            ss: segment(6),
            tr: segment(7),
            ldt: segment(8),
            gdt: sys::KvmDtable {
                base: 0x5000,
                limit: 0x17,
                padding: [0; 3],
            },
            idt: sys::KvmDtable {
                base: 0x6000,
                limit: 0x40f,
                padding: [0; 3],
            },
            cr0: 1,
            cr2: 2,
            cr3: 3,
            cr4: 4,
            cr8: 5,
            efer: 6,
            apic_base: 7,
            interrupt_bitmap: [8, 9, 10, 11],
        });
        let host = HostMsrIndexList::from_validated_raw(&[0x10, 0x1b]);
        let indices = [MsrIndex::new(0x10), MsrIndex::new(0x1b)];
        let policy = GuestMsrAccessPolicy::from_host(&host, &indices).unwrap();
        let values =
            GuestMsrValueSet::from_policy(&policy, &[(indices[0], 0x1111), (indices[1], 0x2222)])
                .unwrap();
        let msrs = GuestMsrSnapshot::from_capture(&policy, &values).unwrap();
        let vcpu = VcpuStateSnapshot {
            registers,
            special_registers,
            msrs,
        };
        let pages = vec![
            BoundedCheckpointPage {
                address: GuestPhysAddr::new(0x30000),
                bytes: vec![0x3a; LONG_MODE_PAGE_SIZE as usize],
            },
            BoundedCheckpointPage {
                address: GuestPhysAddr::new(0x31000),
                bytes: vec![0x4b; LONG_MODE_PAGE_SIZE as usize],
            },
        ];
        (BoundedVcpuPageSetCheckpoint { pages, vcpu }, host)
    }

    #[test]
    fn canonical_round_trip_reconstructs_the_checkpoint_semantically() {
        let (checkpoint, host) = checkpoint();
        let schema = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint).unwrap();
        let encoded = schema.encode().unwrap();
        let decoded = VersionedPageVcpuCheckpointV1::decode(&encoded).unwrap();
        let materialized = decoded.materialize(&host).unwrap();
        assert_eq!(schema, decoded);
        assert_eq!(checkpoint, materialized);
        assert_eq!(encoded.len(), encoded_length(2, 2).unwrap());
    }

    #[test]
    fn magic_version_architecture_and_lengths_are_fail_closed() {
        let (checkpoint, _) = checkpoint();
        let bytes = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint)
            .unwrap()
            .encode()
            .unwrap();

        let mut bad_magic = bytes.clone();
        bad_magic[0] ^= 0xff;
        assert_eq!(
            VersionedPageVcpuCheckpointV1::decode(&bad_magic),
            Err(VersionedPageVcpuCheckpointError::InvalidMagic)
        );

        let mut bad_version = bytes.clone();
        bad_version[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedPageVcpuCheckpointV1::decode(&bad_version),
            Err(VersionedPageVcpuCheckpointError::UnsupportedVersion(2))
        );

        let mut bad_arch = bytes.clone();
        bad_arch[10..12].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedPageVcpuCheckpointV1::decode(&bad_arch),
            Err(VersionedPageVcpuCheckpointError::UnsupportedArchitecture(2))
        );

        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(matches!(
            VersionedPageVcpuCheckpointV1::decode(&trailing),
            Err(VersionedPageVcpuCheckpointError::InvalidTotalLength { .. })
        ));
    }

    #[test]
    fn page_order_alignment_and_reserved_fields_are_validated() {
        let (checkpoint, _) = checkpoint();
        let bytes = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint)
            .unwrap()
            .encode()
            .unwrap();

        let mut unaligned = bytes.clone();
        unaligned[HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&0x30001_u64.to_le_bytes());
        assert_eq!(
            VersionedPageVcpuCheckpointV1::decode(&unaligned),
            Err(VersionedPageVcpuCheckpointError::UnalignedPage(0x30001))
        );

        let mut duplicate = bytes.clone();
        let second = HEADER_LEN + PAGE_RECORD_LEN;
        duplicate[second..second + 8].copy_from_slice(&0x30000_u64.to_le_bytes());
        assert!(matches!(
            VersionedPageVcpuCheckpointV1::decode(&duplicate),
            Err(VersionedPageVcpuCheckpointError::NonCanonicalPageOrder { .. })
        ));

        let mut flags = bytes.clone();
        flags[32..36].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedPageVcpuCheckpointV1::decode(&flags),
            Err(VersionedPageVcpuCheckpointError::NonZeroFlags(1))
        );
    }

    #[test]
    fn invalid_segment_bits_and_msr_order_are_rejected_before_materialization() {
        let (checkpoint, _) = checkpoint();
        let bytes = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint)
            .unwrap()
            .encode()
            .unwrap();
        let special_offset = HEADER_LEN + 2 * PAGE_RECORD_LEN + REGISTER_BLOCK_LEN;
        let mut invalid_present = bytes.clone();
        invalid_present[special_offset + 15] = 2;
        assert!(matches!(
            VersionedPageVcpuCheckpointV1::decode(&invalid_present),
            Err(VersionedPageVcpuCheckpointError::InvalidSegmentField {
                field: "present",
                ..
            })
        ));

        let msr_offset = special_offset + SPECIAL_REGISTER_BLOCK_LEN;
        let mut duplicate_msr = bytes;
        duplicate_msr[msr_offset + MSR_RECORD_LEN..msr_offset + MSR_RECORD_LEN + 4]
            .copy_from_slice(&0x10_u32.to_le_bytes());
        assert!(matches!(
            VersionedPageVcpuCheckpointV1::decode(&duplicate_msr),
            Err(VersionedPageVcpuCheckpointError::NonCanonicalMsrOrder { .. })
        ));
    }

    #[test]
    fn current_host_msr_support_is_revalidated_before_checkpoint_materialization() {
        let (checkpoint, _) = checkpoint();
        let schema = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint).unwrap();
        let incompatible_host = HostMsrIndexList::from_validated_raw(&[0x10]);
        assert!(matches!(
            schema.materialize(&incompatible_host),
            Err(VersionedPageVcpuCheckpointError::HostMsrIncompatible(_))
        ));
    }

    #[test]
    fn truncated_input_never_partially_decodes() {
        let (checkpoint, _) = checkpoint();
        let bytes = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint)
            .unwrap()
            .encode()
            .unwrap();
        for length in [0, 7, HEADER_LEN - 1] {
            assert!(VersionedPageVcpuCheckpointV1::decode(&bytes[..length]).is_err());
        }
    }
}
