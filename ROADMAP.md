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
- composite vCPU state snapshots that own the existing general-register, special-register, and policy-bound MSR snapshots together, with pure component-preserving comparison and bounded non-transactional restore;
- centralized VM-exit dispatch, bounded execution budgets, and the minimal bidirectional debug port-I/O device.

## Phase 41 — composite vCPU state snapshot restore

The current bounded slice adds `Vcpu::restore_state_snapshot()` as a thin orchestration boundary over the existing special-register, general-register, and policy-bound MSR restore paths.

Correctness contract:

- restore accepts only an already-owned `VcpuStateSnapshot`; it introduces no raw KVM representation, new state encoding, or new error taxonomy;
- component restore order is fixed as special registers, general registers, then policy-bound MSRs, matching the repository's existing dependency-aware real-mode initialization order for special versus general registers;
- each existing component restore boundary is attempted at most once;
- a special-register restore failure prevents both later restores; a general-register restore failure occurs after the special-register write and prevents MSR restore; an MSR restore failure occurs after both earlier component writes;
- all component errors propagate unchanged, including existing partial-MSR-write diagnostics;
- the operation performs no retry, rollback, repair, transaction log, or automatic recapture;
- successful completion means only that all three existing component restore paths returned success; it does not by itself prove read-back equality;
- because earlier component writes remain applied when a later component fails, this boundary is explicitly non-transactional and does not claim atomic architectural restore semantics;
- pure regressions lock restore ordering and failure short-circuit behavior, while a KVM-aware regression requires a captured real-mode composite snapshot to restore through the public path and recapture as an exact composite match when KVM is available;
- this slice does not add composite restore-and-verify, rollback, migration compatibility, guest-memory/device capture, atomic/quiesced snapshot semantics, long-mode boot, interrupts, MMIO, SMP, or device expansion.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 41 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further state-verification, execution, or architecture-documentation work. Do not infer that composite restore-and-verify, migration orchestration, long-mode boot, interrupts, MMIO, SMP, or device expansion is automatically next merely because composite capture, comparison, and bounded restore now exist.
