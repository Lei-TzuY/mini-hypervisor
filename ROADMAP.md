# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host capability validation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, and pure policy comparison;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots, pure 18-field reference-to-observed comparison, snapshot-bound restore, and restore-and-verify;
- centralized VM-exit dispatch, bounded execution budgets, and the minimal bidirectional debug port-I/O device.

## Phase 34 — vCPU general-register restore-and-verify

The current bounded slice adds `Vcpu::restore_and_verify_register_snapshot(&VcpuRegisterSnapshot)`.

Correctness contract:

- the input is one complete owned `VcpuRegisterSnapshot`;
- the write delegates to the existing `restore_register_snapshot` path, so KVM_SET_REGS serialization remains single-sourced;
- post-write read-back occurs only after the restore succeeds and delegates to `capture_register_snapshot`;
- verification delegates to the existing pure `VcpuRegisterSnapshot::compare` contract, preserving the canonical 18-field mismatch identities and ordering;
- restore or recapture failures propagate without retry, rollback, or automatic repair;
- if KVM accepts the write but the recaptured register state differs, the operation returns the owned `VcpuRegisterSnapshotComparison` instead of converting the mismatch into a special failure;
- RIP and RFLAGS remain ordinary comparison fields rather than termination or lifecycle verdicts;
- this slice does not restore special registers, MSRs, CPUID, memory, device state, or multiple vCPUs and does not claim migration safety.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 34 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing any additional state-restoration or state-composition work. Do not infer that special-register restore, multi-state snapshot composition, migration orchestration, long-mode boot, interrupts, MMIO, SMP, or device expansion is automatically next merely because general-register restore-and-verify now exists.
