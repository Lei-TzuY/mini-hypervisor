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
- read-only composite vCPU state capture that owns the existing general-register, special-register, and policy-bound MSR snapshots together without introducing raw KVM state;
- centralized VM-exit dispatch, bounded execution budgets, and the minimal bidirectional debug port-I/O device.

## Phase 39 — composite vCPU state snapshot capture

The current bounded slice adds `Vcpu::capture_state_snapshot()` and an owned `VcpuStateSnapshot` that groups the existing general-register, special-register, and policy-bound MSR snapshots behind one read-only capture boundary.

Correctness contract:

- capture accepts an explicit already-validated `GuestMsrAccessPolicy`; the composite layer does not derive, widen, or normalize MSR authority;
- component capture order is fixed as general registers, special registers, then policy-bound MSRs, with each existing capture boundary attempted at most once;
- a general-register capture failure prevents both later captures, while a special-register capture failure prevents the MSR capture; errors propagate unchanged and no retry is performed;
- successful results own all three existing typed snapshots, retain the MSR snapshot's owned policy provenance, and remain usable after the source vCPU and caller policy are dropped;
- the composite type exposes only read-only references to the owned component snapshots and introduces no raw KVM UAPI representation or new unsafe code;
- pure sequencing regressions lock canonical order and short-circuit behavior, while a KVM-aware regression validates owned composition for explicitly initialized real-mode state when KVM is available;
- because the three components are read sequentially through separate existing KVM operations, this slice does not claim an atomic or quiesced point-in-time architectural snapshot;
- this slice does not add composite comparison, composite restore, rollback, migration compatibility, guest-memory/device capture, long-mode boot, interrupts, MMIO, SMP, or device expansion.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 39 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further state-comparison, state-restore, execution, or architecture-documentation work. Do not infer that composite comparison, multi-component restore, migration orchestration, long-mode boot, interrupts, MMIO, SMP, or device expansion is automatically next merely because read-only composite vCPU state capture now exists.
