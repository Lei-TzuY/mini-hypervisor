const VERSIONED_VCPU_STATE_MAGIC: [u8; 8] = *b"MHVVCPU\0";
const VERSIONED_VCPU_STATE_VERSION: u16 = 1;
const VERSIONED_VCPU_STATE_ARCH_X86_64: u16 = 1;
const VCPU_STATE_HEADER_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionedVcpuStateSnapshotError {
    InvalidMagic,
    UnsupportedVersion(u16),
    UnsupportedArchitecture(u16),
    InvalidHeaderLength(u32),
    InvalidTotalLength { declared: u64, actual: usize },
    InvalidMsrCount(u16),
    NonZeroFlags(u16),
    NonZeroReserved(u32),
    LengthOverflow,
    State(VersionedPageVcpuCheckpointError),
}

impl std::fmt::Display for VersionedVcpuStateSnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "vCPU-state schema magic does not match MHVVCPU"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported vCPU-state schema version {version}")
            }
            Self::UnsupportedArchitecture(architecture) => write!(
                f,
                "unsupported vCPU-state schema architecture identifier {architecture}"
            ),
            Self::InvalidHeaderLength(length) => {
                write!(f, "vCPU-state schema header length {length} is not {VCPU_STATE_HEADER_LEN}")
            }
            Self::InvalidTotalLength { declared, actual } => write!(
                f,
                "vCPU-state schema declares total length {declared}, actual byte length is {actual}"
            ),
            Self::InvalidMsrCount(count) => write!(
                f,
                "vCPU-state schema MSR count {count} exceeds bounded limit {VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT}"
            ),
            Self::NonZeroFlags(flags) => {
                write!(f, "vCPU-state schema v1 flags must be zero, got {flags:#x}")
            }
            Self::NonZeroReserved(value) => {
                write!(f, "vCPU-state schema reserved field must be zero, got {value:#x}")
            }
            Self::LengthOverflow => write!(f, "vCPU-state schema length arithmetic overflowed"),
            Self::State(error) => write!(f, "vCPU-state semantic payload is invalid: {error}"),
        }
    }
}

impl std::error::Error for VersionedVcpuStateSnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::State(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VersionedPageVcpuCheckpointError> for VersionedVcpuStateSnapshotError {
    fn from(error: VersionedPageVcpuCheckpointError) -> Self {
        Self::State(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionedVcpuStateSnapshotV1 {
    registers: VcpuRegisterSnapshot,
    special_registers: VcpuSpecialRegisterSnapshot,
    msrs: Vec<(MsrIndex, u64)>,
}

impl VersionedVcpuStateSnapshotV1 {
    pub(crate) fn from_snapshot(
        snapshot: &VcpuStateSnapshot,
    ) -> Result<Self, VersionedVcpuStateSnapshotError> {
        let mut msrs = snapshot
            .msrs()
            .values()
            .values()
            .iter()
            .map(|value| (value.index(), value.value()))
            .collect::<Vec<_>>();
        if msrs.len() > VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT {
            return Err(VersionedVcpuStateSnapshotError::InvalidMsrCount(
                u16::try_from(msrs.len()).unwrap_or(u16::MAX),
            ));
        }
        msrs.sort_unstable_by_key(|(index, _)| index.get());
        validate_msr_order(&msrs)?;
        Ok(Self {
            registers: *snapshot.registers(),
            special_registers: *snapshot.special_registers(),
            msrs,
        })
    }

    #[must_use]
    pub(crate) fn msr_count(&self) -> usize {
        self.msrs.len()
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, VersionedVcpuStateSnapshotError> {
        validate_msr_order(&self.msrs)?;
        if self.msrs.len() > VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT {
            return Err(VersionedVcpuStateSnapshotError::InvalidMsrCount(
                u16::try_from(self.msrs.len()).unwrap_or(u16::MAX),
            ));
        }
        let total_len = vcpu_state_encoded_length(self.msrs.len())?;
        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_VCPU_STATE_MAGIC);
        push_u16(&mut bytes, VERSIONED_VCPU_STATE_VERSION);
        push_u16(&mut bytes, VERSIONED_VCPU_STATE_ARCH_X86_64);
        push_u32(&mut bytes, VCPU_STATE_HEADER_LEN as u32);
        push_u64(&mut bytes, total_len as u64);
        push_u16(
            &mut bytes,
            u16::try_from(self.msrs.len()).expect("bounded vCPU-state MSR count fits u16"),
        );
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);

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

    pub(crate) fn decode(
        bytes: &[u8],
    ) -> Result<Self, VersionedVcpuStateSnapshotError> {
        let mut cursor = Cursor::new(bytes);
        if cursor.take(8)? != VERSIONED_VCPU_STATE_MAGIC {
            return Err(VersionedVcpuStateSnapshotError::InvalidMagic);
        }
        let version = cursor.u16()?;
        if version != VERSIONED_VCPU_STATE_VERSION {
            return Err(VersionedVcpuStateSnapshotError::UnsupportedVersion(version));
        }
        let architecture = cursor.u16()?;
        if architecture != VERSIONED_VCPU_STATE_ARCH_X86_64 {
            return Err(VersionedVcpuStateSnapshotError::UnsupportedArchitecture(
                architecture,
            ));
        }
        let header_len = cursor.u32()?;
        if header_len != VCPU_STATE_HEADER_LEN as u32 {
            return Err(VersionedVcpuStateSnapshotError::InvalidHeaderLength(
                header_len,
            ));
        }
        let declared_total = cursor.u64()?;
        if declared_total != bytes.len() as u64 {
            return Err(VersionedVcpuStateSnapshotError::InvalidTotalLength {
                declared: declared_total,
                actual: bytes.len(),
            });
        }
        let msr_count = cursor.u16()?;
        if usize::from(msr_count) > VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT {
            return Err(VersionedVcpuStateSnapshotError::InvalidMsrCount(msr_count));
        }
        let flags = cursor.u16()?;
        if flags != 0 {
            return Err(VersionedVcpuStateSnapshotError::NonZeroFlags(flags));
        }
        let reserved = cursor.u32()?;
        if reserved != 0 {
            return Err(VersionedVcpuStateSnapshotError::NonZeroReserved(reserved));
        }
        let expected_len = vcpu_state_encoded_length(usize::from(msr_count))?;
        if expected_len != bytes.len() {
            return Err(VersionedVcpuStateSnapshotError::InvalidTotalLength {
                declared: declared_total,
                actual: expected_len,
            });
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
                    field: "vcpu-state.msr.reserved",
                    value: u64::from(reserved),
                }
                .into());
            }
            if let Some(previous) = previous_msr {
                if index <= previous {
                    return Err(VersionedPageVcpuCheckpointError::NonCanonicalMsrOrder {
                        previous,
                        current: index,
                    }
                    .into());
                }
            }
            msrs.push((MsrIndex::new(index), cursor.u64()?));
            previous_msr = Some(index);
        }
        if cursor.remaining() != 0 {
            return Err(VersionedVcpuStateSnapshotError::InvalidTotalLength {
                declared: declared_total,
                actual: cursor.offset,
            });
        }
        Ok(Self {
            registers,
            special_registers,
            msrs,
        })
    }

    pub(crate) fn materialize(
        &self,
        host_msrs: &HostMsrIndexList,
    ) -> Result<VcpuStateSnapshot, VersionedVcpuStateSnapshotError> {
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
        Ok(VcpuStateSnapshot {
            registers: self.registers,
            special_registers: self.special_registers,
            msrs,
        })
    }
}

fn vcpu_state_encoded_length(
    msr_count: usize,
) -> Result<usize, VersionedVcpuStateSnapshotError> {
    let msrs = MSR_RECORD_LEN
        .checked_mul(msr_count)
        .ok_or(VersionedVcpuStateSnapshotError::LengthOverflow)?;
    VCPU_STATE_HEADER_LEN
        .checked_add(REGISTER_BLOCK_LEN)
        .and_then(|length| length.checked_add(SPECIAL_REGISTER_BLOCK_LEN))
        .and_then(|length| length.checked_add(msrs))
        .ok_or(VersionedVcpuStateSnapshotError::LengthOverflow)
}

#[cfg(test)]
mod vcpu_state_schema_tests {
    use super::*;

    #[test]
    fn vcpu_state_header_fails_closed() {
        let registers = VcpuRegisterSnapshot::from_kvm_regs(sys::KvmRegs::default());
        let special_registers =
            VcpuSpecialRegisterSnapshot::from_kvm_sregs(sys::KvmSregs::default());
        let host = HostMsrIndexList::from_validated_raw(&[]);
        let policy = GuestMsrAccessPolicy::from_host(&host, &[]).unwrap();
        let values = GuestMsrValueSet::from_policy(&policy, &[]).unwrap();
        let msrs = GuestMsrSnapshot::from_capture(&policy, &values).unwrap();
        let snapshot = VcpuStateSnapshot {
            registers,
            special_registers,
            msrs,
        };
        let schema = VersionedVcpuStateSnapshotV1::from_snapshot(&snapshot).unwrap();
        let encoded = schema.encode().unwrap();

        let decoded = VersionedVcpuStateSnapshotV1::decode(&encoded).unwrap();
        assert_eq!(decoded.encode().unwrap(), encoded);
        assert_eq!(VERSIONED_VCPU_STATE_VERSION, 1);
        assert_eq!(decoded.msr_count(), 0);

        let mut bad_magic = encoded.clone();
        bad_magic[0] ^= 0xff;
        assert_eq!(
            VersionedVcpuStateSnapshotV1::decode(&bad_magic),
            Err(VersionedVcpuStateSnapshotError::InvalidMagic)
        );

        let mut bad_flags = encoded;
        bad_flags[26] = 1;
        assert_eq!(
            VersionedVcpuStateSnapshotV1::decode(&bad_flags),
            Err(VersionedVcpuStateSnapshotError::NonZeroFlags(1))
        );
    }
}
