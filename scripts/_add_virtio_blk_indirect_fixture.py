from pathlib import Path

fixture = Path("src/portio/virtio_blk_write_readback_fixture.rs")
text = fixture.read_text()

text = text.replace(
    "use super::pci::virtio_blk::VIRTIO_BLK_T_OUT;\n",
    "use super::pci::virtio_blk::{\n    VIRTIO_BLK_T_OUT, VIRTIO_RING_F_INDIRECT_DESC, VIRTQ_DESC_F_INDIRECT,\n};\n",
    1,
)

old = 'pub const VIRTIO_BLK_WRITE_READBACK_PROOF: &[u8; 7] = b"PBWONRD";\n'
assert text.count(old) == 1
text = text.replace(
    old,
    old
    + 'pub const VIRTIO_BLK_INDIRECT_PROOF: &[u8; 8] = b"PIBWONRD";\n'
    + 'pub const VIRTIO_BLK_INDIRECT_TABLE_GPA: u64 = 0x0001_8700;\n',
)

marker = 'const READ_NOTIFY_BARRIER: u8 = b\'N\';\n\n'
enum_text = '''const READ_NOTIFY_BARRIER: u8 = b'N';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DescriptorTopology {
    Direct,
    Indirect,
}

impl DescriptorTopology {
    const fn proof(self) -> &'static [u8] {
        match self {
            Self::Direct => VIRTIO_BLK_WRITE_READBACK_PROOF,
            Self::Indirect => VIRTIO_BLK_INDIRECT_PROOF,
        }
    }

    const fn expected_driver_features(self) -> u64 {
        match self {
            Self::Direct => VIRTIO_F_VERSION_1,
            Self::Indirect => VIRTIO_F_VERSION_1 | VIRTIO_RING_F_INDIRECT_DESC,
        }
    }
}

'''
assert text.count(marker) == 1
text = text.replace(marker, enum_text)

old = '''    proof: Vec<u8>,
    write_completion: VirtioBlkQueueCompletion,
'''
new = '''    proof: Vec<u8>,
    driver_features: u64,
    write_completion: VirtioBlkQueueCompletion,
'''
assert text.count(old) == 1
text = text.replace(old, new)

old = '''    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    #[must_use]
    pub const fn write_completion(&self) -> VirtioBlkQueueCompletion {
'''
new = '''    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    #[must_use]
    pub const fn driver_features(&self) -> u64 {
        self.driver_features
    }

    #[must_use]
    pub const fn write_completion(&self) -> VirtioBlkQueueCompletion {
'''
assert text.count(old) == 1
text = text.replace(old, new)

old = '''pub fn run_virtio_blk_write_readback_guest(
    config: VmConfig,
) -> Result<VirtioBlkWriteReadbackGuestResult, Error> {
    let guest_bytes = build_write_readback_guest();
'''
new = '''pub fn run_virtio_blk_write_readback_guest(
    config: VmConfig,
) -> Result<VirtioBlkWriteReadbackGuestResult, Error> {
    run_write_readback_guest(config, DescriptorTopology::Direct)
}

pub fn run_virtio_blk_indirect_guest(
    config: VmConfig,
) -> Result<VirtioBlkWriteReadbackGuestResult, Error> {
    run_write_readback_guest(config, DescriptorTopology::Indirect)
}

fn run_write_readback_guest(
    config: VmConfig,
    topology: DescriptorTopology,
) -> Result<VirtioBlkWriteReadbackGuestResult, Error> {
    let guest_bytes = build_write_readback_guest(topology);
'''
assert text.count(old) == 1
text = text.replace(old, new)

old = '''    validate_write_readback_io(execution.io_exits())?;
    validate_write_readback_mmio(execution.mmio_exits())?;

    let backing = mmio
'''
new = '''    validate_write_readback_io(execution.io_exits(), topology.proof())?;
    validate_write_readback_mmio(execution.mmio_exits(), topology)?;

    let driver_features = mmio
        .virtio_blk_driver_features_at(VIRTIO_BLK_BAR0_GPA)
        .ok_or_else(|| write_readback_verification_error("virtio-blk driver features unavailable"))?;
    let backing = mmio
'''
assert text.count(old) == 1
text = text.replace(old, new)

old = '''        || request_status[0] != VIRTIO_BLK_S_OK
        || proof.as_slice() != VIRTIO_BLK_WRITE_READBACK_PROOF
'''
new = '''        || request_status[0] != VIRTIO_BLK_S_OK
        || driver_features != topology.expected_driver_features()
        || proof.as_slice() != topology.proof()
'''
assert text.count(old) == 1
text = text.replace(old, new)

old = '''        proof,
        write_completion,
'''
new = '''        proof,
        driver_features,
        write_completion,
'''
assert text.count(old) == 1
text = text.replace(old, new)

text = text.replace(
    'fn validate_write_readback_io(exits: &[PortIoExit]) -> Result<(), Error> {',
    'fn validate_write_readback_io(exits: &[PortIoExit], proof: &[u8]) -> Result<(), Error> {',
    1,
)
text = text.replace('14 + VIRTIO_BLK_WRITE_READBACK_PROOF.len()', '14 + proof.len()', 2)
text = text.replace(
    '.zip(VIRTIO_BLK_WRITE_READBACK_PROOF.iter().copied())',
    '.zip(proof.iter().copied())',
    1,
)

start = text.index('fn validate_write_readback_mmio(')
end = text.index('\nfn write_readback_verification_error', start)
validator = r'''fn validate_write_readback_mmio(
    exits: &[MmioExit],
    topology: DescriptorTopology,
) -> Result<(), Error> {
    let mut expected: Vec<(u64, MmioDirection, u32, Vec<u8>)> = vec![
        (0x300, MmioDirection::Read, 8, Vec::new()),
        (0x14, MmioDirection::Write, 1, vec![VIRTIO_STATUS_ACKNOWLEDGE]),
        (
            0x14,
            MmioDirection::Write,
            1,
            vec![VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER],
        ),
    ];
    if topology == DescriptorTopology::Indirect {
        expected.extend([
            (0x00, MmioDirection::Write, 4, 0_u32.to_le_bytes().to_vec()),
            (0x04, MmioDirection::Read, 4, Vec::new()),
            (0x08, MmioDirection::Write, 4, 0_u32.to_le_bytes().to_vec()),
            (
                0x0c,
                MmioDirection::Write,
                4,
                (VIRTIO_RING_F_INDIRECT_DESC as u32).to_le_bytes().to_vec(),
            ),
        ]);
    }
    expected.extend([
        (0x00, MmioDirection::Write, 4, 1_u32.to_le_bytes().to_vec()),
        (0x04, MmioDirection::Read, 4, Vec::new()),
        (0x08, MmioDirection::Write, 4, 1_u32.to_le_bytes().to_vec()),
        (0x0c, MmioDirection::Write, 4, 1_u32.to_le_bytes().to_vec()),
        (
            0x14,
            MmioDirection::Write,
            1,
            vec![VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK],
        ),
        (0x16, MmioDirection::Write, 2, 0_u16.to_le_bytes().to_vec()),
        (0x18, MmioDirection::Write, 2, VIRTIO_QUEUE_SIZE.to_le_bytes().to_vec()),
        (
            0x20,
            MmioDirection::Write,
            4,
            (VIRTIO_BLK_DESCRIPTOR_GPA as u32).to_le_bytes().to_vec(),
        ),
        (0x24, MmioDirection::Write, 4, 0_u32.to_le_bytes().to_vec()),
        (
            0x28,
            MmioDirection::Write,
            4,
            (VIRTIO_BLK_AVAIL_GPA as u32).to_le_bytes().to_vec(),
        ),
        (0x2c, MmioDirection::Write, 4, 0_u32.to_le_bytes().to_vec()),
        (
            0x30,
            MmioDirection::Write,
            4,
            (VIRTIO_BLK_USED_GPA as u32).to_le_bytes().to_vec(),
        ),
        (0x34, MmioDirection::Write, 4, 0_u32.to_le_bytes().to_vec()),
        (0x1c, MmioDirection::Write, 2, 1_u16.to_le_bytes().to_vec()),
        (
            0x14,
            MmioDirection::Write,
            1,
            vec![VIRTIO_STATUS_ACKNOWLEDGE
                | VIRTIO_STATUS_DRIVER
                | VIRTIO_STATUS_FEATURES_OK
                | VIRTIO_STATUS_DRIVER_OK],
        ),
        (0x14, MmioDirection::Read, 1, Vec::new()),
        (0x100, MmioDirection::Write, 2, 0_u16.to_le_bytes().to_vec()),
        (0x100, MmioDirection::Write, 2, 0_u16.to_le_bytes().to_vec()),
        (VIRTIO_ISR_OFFSET, MmioDirection::Read, 1, Vec::new()),
    ]);

    if exits.len() != expected.len() {
        return Err(write_readback_verification_error(format!(
            "expected {} virtio-blk write/readback MMIO exits, got {}",
            expected.len(),
            exits.len()
        )));
    }
    for (index, (exit, (offset, direction, length, payload))) in
        exits.iter().zip(expected).enumerate()
    {
        if exit.address() != VIRTIO_BLK_BAR0_GPA + offset
            || exit.direction() != direction
            || exit.length() != length
            || exit.write_data() != payload.as_slice()
        {
            return Err(write_readback_verification_error(format!(
                "virtio-blk write/readback MMIO exit {index} mismatch: {exit:?}"
            )));
        }
    }
    Ok(())
}
'''
text = text[:start] + validator + text[end:]

text = text.replace(
    'fn build_write_readback_guest() -> Vec<u8> {',
    'fn build_write_readback_guest(topology: DescriptorTopology) -> Vec<u8> {',
    1,
)

old = '''    emit_mmio_dword_write(&mut code, 0x00, 1);
    code.extend_from_slice(&[0x8b, 0x43, 0x04]);
    emit_cmp_eax(&mut code, 1);
    emit_mmio_dword_write(&mut code, 0x08, 1);
    emit_mmio_dword_write(&mut code, 0x0c, 1);
'''
new = '''    if topology == DescriptorTopology::Indirect {
        emit_mmio_dword_write(&mut code, 0x00, 0);
        code.extend_from_slice(&[0x8b, 0x43, 0x04]);
        emit_cmp_eax(&mut code, VIRTIO_RING_F_INDIRECT_DESC as u32);
        emit_mmio_dword_write(&mut code, 0x08, 0);
        emit_mmio_dword_write(&mut code, 0x0c, VIRTIO_RING_F_INDIRECT_DESC as u32);
    }
    emit_mmio_dword_write(&mut code, 0x00, 1);
    code.extend_from_slice(&[0x8b, 0x43, 0x04]);
    emit_cmp_eax(&mut code, 1);
    emit_mmio_dword_write(&mut code, 0x08, 1);
    emit_mmio_dword_write(&mut code, 0x0c, 1);
'''
assert text.count(old) == 1
text = text.replace(old, new)

old = '''    emit_debug(&mut code, b'B');

    emit_write_request_setup(&mut code);
'''
new = '''    if topology == DescriptorTopology::Indirect {
        emit_debug(&mut code, b'I');
    }
    emit_debug(&mut code, b'B');

    emit_write_request_setup(&mut code, topology);
'''
assert text.count(old) == 1
text = text.replace(old, new)
text = text.replace('    emit_readback_request_setup(&mut code);', '    emit_readback_request_setup(&mut code, topology);', 1)
text = text.replace(
    'fn emit_write_request_setup(code: &mut Vec<u8>) {\n    emit_request_descriptors(code, VIRTQ_DESC_F_NEXT);',
    'fn emit_write_request_setup(code: &mut Vec<u8>, topology: DescriptorTopology) {\n    emit_request_descriptors(code, VIRTQ_DESC_F_NEXT, topology);',
    1,
)
text = text.replace(
    'fn emit_readback_request_setup(code: &mut Vec<u8>) {\n    emit_request_descriptors(code, VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE);',
    'fn emit_readback_request_setup(code: &mut Vec<u8>, topology: DescriptorTopology) {\n    emit_request_descriptors(code, VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE, topology);',
    1,
)

start = text.index('fn emit_request_descriptors(')
end = text.index('\nfn emit_request_header', start)
descriptors = r'''fn emit_request_descriptors(
    code: &mut Vec<u8>,
    data_flags: u16,
    topology: DescriptorTopology,
) {
    if topology == DescriptorTopology::Indirect {
        emit_movabs(code, 7, VIRTIO_BLK_DESCRIPTOR_GPA);
        code.extend_from_slice(&[0x48, 0xc7, 0x07]);
        code.extend_from_slice(&(VIRTIO_BLK_INDIRECT_TABLE_GPA as u32).to_le_bytes());
        code.extend_from_slice(&[0xc7, 0x47, 0x08, 0x30, 0x00, 0x00, 0x00]);
        code.extend_from_slice(&[0xc7, 0x47, 0x0c]);
        code.extend_from_slice(&u32::from(VIRTQ_DESC_F_INDIRECT).to_le_bytes());

        emit_movabs(code, 7, VIRTIO_BLK_INDIRECT_TABLE_GPA);
        code.extend_from_slice(&[0x48, 0xc7, 0x07]);
        code.extend_from_slice(&(VIRTIO_BLK_HEADER_GPA as u32).to_le_bytes());
        code.extend_from_slice(&[0xc7, 0x47, 0x08, 0x10, 0x00, 0x00, 0x00]);
        code.extend_from_slice(&[0xc7, 0x47, 0x0c]);
        code.extend_from_slice(&(u32::from(VIRTQ_DESC_F_NEXT) | (1_u32 << 16)).to_le_bytes());

        code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x10]);
        code.extend_from_slice(&(VIRTIO_BLK_DATA_GPA as u32).to_le_bytes());
        code.extend_from_slice(&[0xc7, 0x47, 0x18]);
        code.extend_from_slice(&(VIRTIO_BLK_SECTOR_SIZE as u32).to_le_bytes());
        code.extend_from_slice(&[0xc7, 0x47, 0x1c]);
        code.extend_from_slice(&(u32::from(data_flags) | (2_u32 << 16)).to_le_bytes());

        code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x20]);
        code.extend_from_slice(&(VIRTIO_BLK_STATUS_GPA as u32).to_le_bytes());
        code.extend_from_slice(&[0xc7, 0x47, 0x28, 0x01, 0x00, 0x00, 0x00]);
        code.extend_from_slice(&[0xc7, 0x47, 0x2c]);
        code.extend_from_slice(&u32::from(VIRTQ_DESC_F_WRITE).to_le_bytes());
        return;
    }

    emit_movabs(code, 7, VIRTIO_BLK_DESCRIPTOR_GPA);
    code.extend_from_slice(&[0x48, 0xc7, 0x07]);
    code.extend_from_slice(&(VIRTIO_BLK_HEADER_GPA as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x08, 0x10, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xc7, 0x47, 0x0c]);
    let descriptor0_tail = u32::from(VIRTQ_DESC_F_NEXT) | (1_u32 << 16);
    code.extend_from_slice(&descriptor0_tail.to_le_bytes());

    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x10]);
    code.extend_from_slice(&(VIRTIO_BLK_DATA_GPA as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x18]);
    code.extend_from_slice(&(VIRTIO_BLK_SECTOR_SIZE as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x1c]);
    let descriptor1_tail = u32::from(data_flags) | (2_u32 << 16);
    code.extend_from_slice(&descriptor1_tail.to_le_bytes());

    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x20]);
    code.extend_from_slice(&(VIRTIO_BLK_STATUS_GPA as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x28, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xc7, 0x47, 0x2c]);
    code.extend_from_slice(&u32::from(VIRTQ_DESC_F_WRITE).to_le_bytes());
}
'''
text = text[:start] + descriptors + text[end:]

fixture.write_text(text)

parent = Path("src/portio/virtio_blk_fixture.rs")
ptext = parent.read_text()
old = '''pub use write_readback::{
    deterministic_write_readback_sector, run_virtio_blk_write_readback_guest,
    VirtioBlkWriteReadbackGuestResult, VIRTIO_BLK_WRITE_READBACK_PROOF,
};
'''
new = '''pub use write_readback::{
    deterministic_write_readback_sector, run_virtio_blk_indirect_guest,
    run_virtio_blk_write_readback_guest, VirtioBlkWriteReadbackGuestResult,
    VIRTIO_BLK_INDIRECT_PROOF, VIRTIO_BLK_INDIRECT_TABLE_GPA,
    VIRTIO_BLK_WRITE_READBACK_PROOF,
};
'''
assert ptext.count(old) == 1
parent.write_text(ptext.replace(old, new))

Path("src/bin/virtio-blk-indirect.rs").write_text(r'''use mini_hypervisor::config::VmConfig;
use mini_hypervisor::portio::pci::virtio::VIRTIO_F_VERSION_1;
use mini_hypervisor::portio::pci::virtio_blk::{
    VIRTIO_BLK_S_OK, VIRTIO_RING_F_INDIRECT_DESC,
};
use mini_hypervisor::portio::virtio_blk_fixture::{
    deterministic_write_readback_sector, run_virtio_blk_indirect_guest,
    VIRTIO_BLK_INDIRECT_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_virtio_blk_indirect_guest(VmConfig::default()) {
        Ok(result) => {
            println!("virtio-blk indirect driver features: {:#x}", result.driver_features());
            println!(
                "virtio-blk indirect write completion: {}/{}/{}",
                result.write_completion().descriptor_id(),
                result.write_completion().length(),
                result.write_completion().sector()
            );
            println!(
                "virtio-blk indirect readback completion: {}/{}/{}",
                result.read_completion().descriptor_id(),
                result.read_completion().length(),
                result.read_completion().sector()
            );
            println!(
                "virtio-blk indirect used: {}/{}/{}/{}/{}",
                result.used_idx(),
                result.first_used_id(),
                result.first_used_len(),
                result.second_used_id(),
                result.second_used_len()
            );
            println!("virtio-blk indirect request status: {}", result.request_status());
            println!("virtio-blk indirect proof: {:?}", result.proof());
            println!("virtio-blk indirect port-I/O exits: {}", result.io_exits().len());
            println!("virtio-blk indirect MMIO exits: {}", result.mmio_exits().len());
            println!("{}", result.report());

            let expected = deterministic_write_readback_sector();
            let expected_features = VIRTIO_F_VERSION_1 | VIRTIO_RING_F_INDIRECT_DESC;
            if result.driver_features() == expected_features
                && result.write_completion().descriptor_id() == 0
                && result.write_completion().length() == 1
                && result.write_completion().sector() == 0
                && result.read_completion().descriptor_id() == 0
                && result.read_completion().length() == 513
                && result.read_completion().sector() == 0
                && result.used_idx() == 2
                && result.first_used_id() == 0
                && result.first_used_len() == 1
                && result.second_used_id() == 0
                && result.second_used_len() == 513
                && result.request_status() == VIRTIO_BLK_S_OK
                && result.backing() == expected
                && result.readback() == expected
                && result.proof() == VIRTIO_BLK_INDIRECT_PROOF
                && result.io_exits().len() == 22
                && result.mmio_exits().len() == 26
            {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: virtio-blk indirect proof did not match expected state");
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("error: {error}");
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                eprintln!("caused by: {cause}");
                source = cause.source();
            }
            ExitCode::FAILURE
        }
    }
}
''')

Path("tests/virtio_blk_indirect_guest.rs").write_text(r'''use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::pci::virtio::VIRTIO_F_VERSION_1;
use mini_hypervisor::portio::pci::virtio_blk::{
    VIRTIO_BLK_S_OK, VIRTIO_RING_F_INDIRECT_DESC,
};
use mini_hypervisor::portio::virtio_blk_fixture::{
    deterministic_write_readback_sector, run_virtio_blk_indirect_guest,
    VIRTIO_BLK_INDIRECT_PROOF,
};
use mini_hypervisor::vcpu::{MmioDirection, VcpuExit};

#[test]
fn guest_negotiates_indirect_feature_and_round_trips_through_indirect_tables() {
    match run_virtio_blk_indirect_guest(VmConfig::default()) {
        Ok(result) => {
            let expected = deterministic_write_readback_sector();
            assert_eq!(
                result.driver_features(),
                VIRTIO_F_VERSION_1 | VIRTIO_RING_F_INDIRECT_DESC
            );
            assert_eq!(result.write_completion().descriptor_id(), 0);
            assert_eq!(result.write_completion().length(), 1);
            assert_eq!(result.read_completion().descriptor_id(), 0);
            assert_eq!(result.read_completion().length(), 513);
            assert_eq!(result.used_idx(), 2);
            assert_eq!(result.first_used_id(), 0);
            assert_eq!(result.second_used_id(), 0);
            assert_eq!(result.request_status(), VIRTIO_BLK_S_OK);
            assert_eq!(result.backing(), expected);
            assert_eq!(result.readback(), expected);
            assert_eq!(result.proof(), VIRTIO_BLK_INDIRECT_PROOF);
            assert_eq!(result.io_exits().len(), 22);
            assert_eq!(result.mmio_exits().len(), 26);

            let low_select = &result.mmio_exits()[3];
            let low_read = &result.mmio_exits()[4];
            let low_driver_select = &result.mmio_exits()[5];
            let low_driver_write = &result.mmio_exits()[6];
            assert_eq!(low_select.direction(), MmioDirection::Write);
            assert_eq!(low_select.write_data(), &0_u32.to_le_bytes());
            assert_eq!(low_read.direction(), MmioDirection::Read);
            assert_eq!(low_driver_select.write_data(), &0_u32.to_le_bytes());
            assert_eq!(
                low_driver_write.write_data(),
                &(VIRTIO_RING_F_INDIRECT_DESC as u32).to_le_bytes()
            );
            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping virtio-blk indirect integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("virtio-blk indirect guest failed unexpectedly: {error}"),
    }
}
''')

Path(".github/workflows/virtio-blk-indirect.yml").write_text(r'''name: Strict KVM virtio-blk indirect

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

jobs:
  virtio-blk-indirect:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
      - name: Strict KVM virtio-blk indirect proof
        shell: bash
        run: |
          test -e /dev/kvm
          sudo chmod a+rw /dev/kvm
          output="$(cargo run --quiet --bin virtio-blk-indirect)"
          printf '%s\n' "$output"
          grep -F 'virtio-blk indirect driver features: 0x110000000' <<<"$output"
          grep -F 'virtio-blk indirect write completion: 0/1/0' <<<"$output"
          grep -F 'virtio-blk indirect readback completion: 0/513/0' <<<"$output"
          grep -F 'virtio-blk indirect used: 2/0/1/0/513' <<<"$output"
          grep -F 'virtio-blk indirect request status: 0' <<<"$output"
          grep -F 'virtio-blk indirect proof: [80, 73, 66, 87, 79, 78, 82, 68]' <<<"$output"
          grep -F 'virtio-blk indirect port-I/O exits: 22' <<<"$output"
          grep -F 'virtio-blk indirect MMIO exits: 26' <<<"$output"
          report="$(grep -F 'vCPU 0 exit Hlt: rip=' <<<"$output")"
          rflags_hex="${report##*rflags=}"
          [[ "$rflags_hex" =~ ^0x[0-9a-fA-F]+$ ]]
          rflags=$((rflags_hex))
          (( (rflags & 0x2) == 0x2 ))
''')
