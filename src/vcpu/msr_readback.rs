use super::{vcpu_operation, Vcpu};
use crate::error::Error;
use crate::kvm::msr::MsrIndex;
use crate::kvm::sys;
use std::io;
use std::os::fd::AsRawFd;

const VCPU_MSR_READBACK_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuMsrValue {
    index: MsrIndex,
    value: u64,
}

impl VcpuMsrValue {
    const fn new(index: MsrIndex, value: u64) -> Self {
        Self { index, value }
    }

    #[must_use]
    pub const fn index(self) -> MsrIndex {
        self.index
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcpuMsrValues {
    values: Vec<VcpuMsrValue>,
}

impl VcpuMsrValues {
    fn from_values(values: Vec<VcpuMsrValue>) -> Self {
        Self { values }
    }

    #[must_use]
    pub fn values(&self) -> &[VcpuMsrValue] {
        &self.values
    }
}

impl Vcpu {
    pub fn msrs(&self, indices: &[MsrIndex]) -> Result<VcpuMsrValues, Error> {
        if indices.is_empty() {
            return Ok(VcpuMsrValues::from_values(Vec::new()));
        }

        let mut request = prepare_vcpu_msr_request(indices).map_err(|source| {
            vcpu_operation(self.id, "validate KVM_GET_MSRS request", source)
        })?;
        let returned = sys::get_msrs(self.fd.as_raw_fd(), &mut request)
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_MSRS", source))?;
        decode_vcpu_msr_response(indices, returned, &request.entries[..indices.len()]).map_err(
            |source| vcpu_operation(self.id, "validate KVM_GET_MSRS response", source),
        )
    }
}

fn prepare_vcpu_msr_request(
    indices: &[MsrIndex],
) -> io::Result<sys::KvmMsrs<VCPU_MSR_READBACK_CAPACITY>> {
    if indices.len() > VCPU_MSR_READBACK_CAPACITY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "vCPU KVM_GET_MSRS request count {} exceeds bounded capacity {}",
                indices.len(),
                VCPU_MSR_READBACK_CAPACITY
            ),
        ));
    }

    let mut request = sys::KvmMsrs::<VCPU_MSR_READBACK_CAPACITY>::new();
    request.nmsrs =
        u32::try_from(indices.len()).expect("bounded vCPU MSR request count always fits u32");
    for (entry, index) in request.entries[..indices.len()]
        .iter_mut()
        .zip(indices.iter().copied())
    {
        entry.index = index.get();
    }
    Ok(request)
}

fn decode_vcpu_msr_response(
    expected: &[MsrIndex],
    returned: usize,
    entries: &[sys::KvmMsrEntry],
) -> io::Result<VcpuMsrValues> {
    if returned != expected.len() {
        let detail = if returned < expected.len() {
            format!(
                "KVM_GET_MSRS returned {returned} of {} requested vCPU MSRs; first unread index {:#x}",
                expected.len(),
                expected[returned].get()
            )
        } else {
            format!(
                "KVM_GET_MSRS returned {returned} vCPU MSRs after {} were requested",
                expected.len()
            )
        };
        return Err(io::Error::new(io::ErrorKind::InvalidData, detail));
    }

    if entries.len() < expected.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "KVM_GET_MSRS response buffer has {} entries for {} requested vCPU MSRs",
                entries.len(),
                expected.len()
            ),
        ));
    }

    let mut values = Vec::with_capacity(expected.len());
    for (position, (expected_index, entry)) in expected
        .iter()
        .copied()
        .zip(entries.iter().copied())
        .enumerate()
    {
        if entry.index != expected_index.get() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "KVM_GET_MSRS changed vCPU MSR index at entry {position}: expected {:#x}, got {:#x}",
                    expected_index.get(),
                    entry.index
                ),
            ));
        }
        values.push(VcpuMsrValue::new(expected_index, entry.data));
    }

    Ok(VcpuMsrValues::from_values(values))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: u32, data: u64) -> sys::KvmMsrEntry {
        sys::KvmMsrEntry {
            index,
            reserved: 0,
            data,
        }
    }

    #[test]
    fn request_builder_preserves_caller_order_and_zeroes_non_index_fields() {
        let indices = [
            MsrIndex::new(0x10a),
            MsrIndex::new(0x3a),
            MsrIndex::new(0x10a),
        ];

        let request = prepare_vcpu_msr_request(&indices).unwrap();

        assert_eq!(request.nmsrs, 3);
        assert_eq!(request.pad, 0);
        assert_eq!(request.entries[0], entry(0x10a, 0));
        assert_eq!(request.entries[1], entry(0x3a, 0));
        assert_eq!(request.entries[2], entry(0x10a, 0));
    }

    #[test]
    fn request_builder_rejects_count_above_project_bound() {
        let indices = vec![MsrIndex::new(0); VCPU_MSR_READBACK_CAPACITY + 1];

        let error = prepare_vcpu_msr_request(&indices).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn response_decoder_preserves_exact_index_order_and_values() {
        let expected = [MsrIndex::new(0x10a), MsrIndex::new(0x3a)];
        let entries = [
            entry(0x10a, 0x1111_2222_3333_4444),
            entry(0x3a, 0xaaaa_bbbb_cccc_dddd),
        ];

        let values = decode_vcpu_msr_response(&expected, 2, &entries).unwrap();

        assert_eq!(
            values.values(),
            &[
                VcpuMsrValue::new(expected[0], 0x1111_2222_3333_4444),
                VcpuMsrValue::new(expected[1], 0xaaaa_bbbb_cccc_dddd),
            ]
        );
    }

    #[test]
    fn response_decoder_accepts_empty_response() {
        let values = decode_vcpu_msr_response(&[], 0, &[]).unwrap();
        assert!(values.values().is_empty());
    }

    #[test]
    fn response_decoder_rejects_partial_completion_and_identifies_first_unread_index() {
        let expected = [MsrIndex::new(0x10), MsrIndex::new(0x1b)];
        let entries = [entry(0x10, 1), entry(0x1b, 2)];

        let error = decode_vcpu_msr_response(&expected, 1, &entries).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("first unread index 0x1b"));
    }

    #[test]
    fn response_decoder_rejects_completion_above_requested_count() {
        let expected = [MsrIndex::new(0x10)];
        let entries = [entry(0x10, 1)];

        let error = decode_vcpu_msr_response(&expected, 2, &entries).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn response_decoder_rejects_short_backing_slice() {
        let expected = [MsrIndex::new(0x10), MsrIndex::new(0x1b)];
        let entries = [entry(0x10, 1)];

        let error = decode_vcpu_msr_response(&expected, 2, &entries).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn response_decoder_rejects_changed_index_before_publishing_values() {
        let expected = [MsrIndex::new(0x10), MsrIndex::new(0x1b)];
        let entries = [entry(0x10, 1), entry(0x3a, 2)];

        let error = decode_vcpu_msr_response(&expected, 2, &entries).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error
            .to_string()
            .contains("entry 1: expected 0x1b, got 0x3a"));
    }
}
