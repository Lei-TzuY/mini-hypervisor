# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host capability validation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, and pure policy comparison;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots, pure 18-field reference-to-observed comparison, snapshot-bound restore, and restore-and-verify;
- owned vCPU special-register snapshots covering segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM UAPI padding, plus pure deterministic semantic-field comparison, snapshot-bound restore, and restore-and-verify;
- read-only composite vCPU state capture that owns the existing general-register, special-register, and policy-bound MSR snapshots together without introducing raw KVM state, plus pure component-preserving composite comparison;
- centralized VM-exit dispatch, bounded execution budgets, and the minimal bidirectional debug port-I/O device.

## Phase 40 — composite vCPU state snapshot comparison

The current bounded slice adds pure `VcpuStateSnapshot::compare()` and an owned `VcpuStateSnapshotComparison` that composes the existing general-register, special-register, and policy-bound MSR comparison boundaries without introducing new mismatch semantics.

Correctness contract:

- comparison is pure Rust over two already-owned `VcpuStateSnapshot` values and performs no KVM operation, retry, mutation, or state recapture;
- component comparison order is fixed as general registers, special registers, then policy-bound MSRs, with each existing comparison boundary invoked exactly once;
- the composite result preserves the three existing typed comparison values and exposes them read-only instead of flattening or renumbering component mismatches;
- general-register field identity and ordering remain owned by `VcpuRegisterSnapshot::compare()`;
- special-register semantic-field identity and ordering remain owned by `VcpuSpecialRegisterSnapshot::compare()`;
- MSR policy equality, value mismatch identity, and the rule that policy mismatch suppresses value-level comparison remain owned by `GuestMsrSnapshot::compare()`;
- `VcpuStateSnapshotComparison::is_exact_match()` is true if and only if all three component comparisons are exact matches;
- pure sequencing regression locks canonical comparison order, while the KVM-aware regression requires identical composite captures to compare exact and a controlled RIP-only change to surface only through the general-register comparison when KVM is available;
- this slice does not add composite restore, rollback, migration compatibility, guest-memory/device capture, atomic/quiesced snapshot semantics, long-mode boot, interrupts, MMIO, SMP, or device expansion.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 40 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further state-restore, execution, or architecture-documentation work. Do not infer that multi-component restore, migration orchestration, long-mode boot, interrupts, MMIO, SMP, or device expansion is automatically next merely because composite capture and comparison now exist.
