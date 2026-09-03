# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host capability validation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, and pure policy comparison;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots plus pure 18-field reference-to-observed comparison;
- centralized VM-exit dispatch, bounded execution budgets, and the minimal bidirectional debug port-I/O device.

## Phase 33 — snapshot-bound vCPU general-register restore

The current bounded slice adds `Vcpu::restore_register_snapshot(&VcpuRegisterSnapshot)`.

Correctness contract:

- the restore input is a complete owned `VcpuRegisterSnapshot`, not an arbitrary raw register structure;
- all 18 captured general-register fields are serialized exactly back to `KvmRegs`: RAX, RBX, RCX, RDX, RSI, RDI, RSP, RBP, R8-R15, RIP, and RFLAGS;
- restore performs one existing `KVM_SET_REGS` operation and returns success only when that ioctl succeeds;
- a KVM failure remains the existing structured vCPU-operation error with operation `KVM_SET_REGS`;
- the KVM-aware regression changes general-register state, restores an earlier owned snapshot, recaptures it, and requires the existing snapshot comparison to report an exact match;
- this slice does not restore special registers, MSRs, CPUID, memory, device state, or multiple vCPUs and does not claim migration safety.

## Next bounded slice

Add **restore-and-verify for one owned vCPU general-register snapshot**.

The intended API should take an existing `VcpuRegisterSnapshot`, perform exactly one existing snapshot-bound restore, recapture general registers through `Vcpu::capture_register_snapshot`, and return the existing owned `VcpuRegisterSnapshotComparison` between the requested snapshot and the observed post-restore state.

Correctness requirements:

- delegate the write to `restore_register_snapshot`; do not duplicate `KVM_SET_REGS` serialization logic;
- perform read-back only after the restore succeeds;
- delegate verification to the existing pure `VcpuRegisterSnapshot::compare` contract so the canonical 18-field diagnostics remain unchanged;
- propagate restore or recapture errors without retry or rollback;
- return an owned comparison even when the kernel accepted the write but post-write state differs;
- do not interpret RIP/RFLAGS mismatches as a termination verdict;
- do not add `KVM_SET_SREGS`, special-register snapshots, MSR/CPUID/device composition, multi-vCPU orchestration, rollback, automatic repair, migration-safety claims, long-mode boot, interrupts, MMIO, SMP, or device expansion in the same slice.

After that slice is integrated and verified, re-inspect live repository state before choosing any broader state-composition work.
