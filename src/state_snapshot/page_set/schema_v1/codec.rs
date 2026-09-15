impl VersionedPageVcpuCheckpointV1 {
    pub fn from_checkpoint(
        checkpoint: &BoundedVcpuPageSetCheckpoint,
    ) -> Result<Self, VersionedPageVcpuCheckpointError> {
        validate_pages(checkpoint.pages())?;
        let mut msrs = checkpoint
            .vcpu()
            .msrs()
            .values()
            .values()
            .iter()
            .map(|value| (value.index(), value.value()))
            .collect::<Vec<_>>();
        if msrs.len() > VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT {
            return Err(VersionedPageVcpuCheckpointError::InvalidMsrCount(
                u16::try_from(msrs.len()).unwrap_or(u16::MAX),
            ));
        }
        msrs.sort_unstable_by_key(|(index, _)| index.get());
        validate_msr_order(&msrs)?;
        Ok(Self {
            pages: checkpoint.pages().to_vec(),
            registers: *checkpoint.vcpu().registers(),
            special_registers: *checkpoint.vcpu().special_registers(),
            msrs,
        })
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        VERSIONED_PAGE_VCPU_CHECKPOINT_VERSION
    }

    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPage] {
        &self.pages
    }

    #[must_use]
    pub fn msr_count(&self) -> usize {
        self.msrs.len()
    }

    pub fn encode(&self) -> Result<Vec<u8>, VersionedPageVcpuCheckpointError> {
        validate_pages(&self.pages)?;
        validate_msr_order(&self.msrs)?;
        if self.msrs.len() > VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT {
            return Err(VersionedPageVcpuCheckpointError::InvalidMsrCount(
                u16::try_from(self.msrs.len()).unwrap_or(u16::MAX),
            ));
        }
        let total_len = encoded_length(self.pages.len(), self.msrs.len())?;
        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_PAGE_VCPU_CHECKPOINT_MAGIC);
        push_u16(&mut bytes, VERSIONED_PAGE_VCPU_CHECKPOINT_VERSION);
        push_u16(&mut bytes, VERSIONED_PAGE_VCPU_CHECKPOINT_ARCH_X86_64);
        push_u32(&mut bytes, HEADER_LEN as u32);
        push_u64(&mut bytes, total_len as u64);
        push_u32(&mut bytes, LONG_MODE_PAGE_SIZE as u32);
        push_u16(
            &mut bytes,
            u16::try_from(self.pages.len()).expect("bounded checkpoint page count fits u16"),
        );
        push_u16(
            &mut bytes,
            u16::try_from(self.msrs.len()).expect("bounded checkpoint MSR count fits u16"),
        );
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);

        for page in &self.pages {
            push_u64(&mut bytes, page.address().get());
            bytes.extend_from_slice(page.bytes());
        }
        encode_registers(&mut bytes, &self.registers);
        encode_special_registers(&mut bytes, &self.special_registers);
        for (index, value) in &self.msrs {
            push_u32(&mut bytes, index.get());
            push_u32(&mut bytes, 0);
            push_u64(&mut bytes, *value);
        }
        debug_assert_eq!(bytes.len(), total_len);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, VersionedPageVcpuCheckpointError> {
        let mut cursor = Cursor::new(bytes);
        let magic = cursor.take(8)?;
        if magic != VERSIONED_PAGE_VCPU_CHECKPOINT_MAGIC {
            return Err(VersionedPageVcpuCheckpointError::InvalidMagic);
        }
        let version = cursor.u16()?;
        if version != VERSIONED_PAGE_VCPU_CHECKPOINT_VERSION {
            return Err(VersionedPageVcpuCheckpointError::UnsupportedVersion(
                version,
            ));
        }
        let architecture = cursor.u16()?;
        if architecture != VERSIONED_PAGE_VCPU_CHECKPOINT_ARCH_X86_64 {
            return Err(VersionedPageVcpuCheckpointError::UnsupportedArchitecture(
                architecture,
            ));
        }
        let header_len = cursor.u32()?;
        if header_len != HEADER_LEN as u32 {
            return Err(VersionedPageVcpuCheckpointError::InvalidHeaderLength(
                header_len,
            ));
        }
        let declared_total = cursor.u64()?;
        if declared_total != bytes.len() as u64 {
            return Err(VersionedPageVcpuCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: bytes.len(),
            });
        }
        let page_size = cursor.u32()?;
        if page_size != LONG_MODE_PAGE_SIZE as u32 {
            return Err(VersionedPageVcpuCheckpointError::InvalidPageSize(page_size));
        }
        let page_count = cursor.u16()?;
        if page_count == 0 || usize::from(page_count) > BOUNDED_CHECKPOINT_PAGE_SET_LIMIT {
            return Err(VersionedPageVcpuCheckpointError::InvalidPageCount(
                page_count,
            ));
        }
        let msr_count = cursor.u16()?;
        if usize::from(msr_count) > VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT {
            return Err(VersionedPageVcpuCheckpointError::InvalidMsrCount(msr_count));
        }
        let flags = cursor.u32()?;
        if flags != 0 {
            return Err(VersionedPageVcpuCheckpointError::NonZeroFlags(flags));
        }
        let reserved = cursor.u32()?;
        if reserved != 0 {
            return Err(VersionedPageVcpuCheckpointError::NonZeroReserved {
                field: "header.reserved",
                value: u64::from(reserved),
            });
        }
        let expected_len = encoded_length(usize::from(page_count), usize::from(msr_count))?;
        if expected_len != bytes.len() {
            return Err(VersionedPageVcpuCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: expected_len,
            });
        }

        let mut pages = Vec::with_capacity(usize::from(page_count));
        let mut previous_page = None;
        for _ in 0..page_count {
            let address = cursor.u64()?;
            validate_page_address(address)?;
            if let Some(previous) = previous_page {
                if address <= previous {
                    return Err(VersionedPageVcpuCheckpointError::NonCanonicalPageOrder {
                        previous,
                        current: address,
                    });
                }
            }
            let page_bytes = cursor.take(LONG_MODE_PAGE_SIZE as usize)?.to_vec();
            pages.push(BoundedCheckpointPage {
                address: GuestPhysAddr::new(address),
                bytes: page_bytes,
            });
            previous_page = Some(address);
        }

        let registers = VcpuRegisterSnapshot::from_kvm_regs(decode_registers(&mut cursor)?);
        let special_registers =
            VcpuSpecialRegisterSnapshot::from_kvm_sregs(decode_special_registers(&mut cursor)?);
        let mut msrs = Vec::with_capacity(usize::from(msr_count));
        let mut previous_msr = None;
        for _ in 0..msr_count {
            let index = cursor.u32()?;
            let reserved = cursor.u32()?;
            if reserved != 0 {
                return Err(VersionedPageVcpuCheckpointError::NonZeroReserved {
                    field: "msr.reserved",
                    value: u64::from(reserved),
                });
            }
            if let Some(previous) = previous_msr {
                if index <= previous {
                    return Err(VersionedPageVcpuCheckpointError::NonCanonicalMsrOrder {
                        previous,
                        current: index,
                    });
                }
            }
            let value = cursor.u64()?;
            msrs.push((MsrIndex::new(index), value));
            previous_msr = Some(index);
        }
        if cursor.remaining() != 0 {
            return Err(VersionedPageVcpuCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: cursor.offset,
            });
        }
        Ok(Self {
            pages,
            registers,
            special_registers,
            msrs,
        })
    }

    pub fn materialize(
        &self,
        host_msrs: &HostMsrIndexList,
    ) -> Result<BoundedVcpuPageSetCheckpoint, VersionedPageVcpuCheckpointError> {
        validate_pages(&self.pages)?;
        validate_msr_order(&self.msrs)?;
        let indices = self
            .msrs
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>();
        let policy = GuestMsrAccessPolicy::from_host(host_msrs, &indices).map_err(|error| {
            VersionedPageVcpuCheckpointError::HostMsrIncompatible(error.to_string())
        })?;
        let values = GuestMsrValueSet::from_policy(&policy, &self.msrs)
            .map_err(|error| VersionedPageVcpuCheckpointError::MsrValueSet(error.to_string()))?;
        let msrs = GuestMsrSnapshot::from_capture(&policy, &values)
            .map_err(|error| VersionedPageVcpuCheckpointError::MsrSnapshot(error.to_string()))?;
        let vcpu = VcpuStateSnapshot {
            registers: self.registers,
            special_registers: self.special_registers,
            msrs,
        };
        Ok(BoundedVcpuPageSetCheckpoint {
            pages: self.pages.clone(),
            vcpu,
        })
    }
}

