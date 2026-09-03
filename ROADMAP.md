# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host capability validation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, pure policy comparison, and a deterministic CLI guest-proof fixture;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- composite CPU-model candidates that own the configured guest CPUID policy together with the immutable host MSR model candidate, including backend materialization, component-preserving pure comparison, and aggregate exactness;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots, pure 18-field reference-to-observed comparison, snapshot-bound restore, and restore-and-verify;
- owned vCPU special-register snapshots covering segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM UAPI padding, plus pure deterministic semantic-field comparison, snapshot-bound restore, and restore-and-verify;
- composite vCPU state snapshots that own the existing general-register, special-register, and policy-bound MSR snapshots together, with pure component-preserving comparison, read-only snapshot-bound verification, bounded non-transactional restore, restore-and-verify, and a deterministic public/CLI round-trip fixture;
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion diagnostics, and unhandled-exit diagnostics, plus the minimal bidirectional debug port-I/O device.

## Phase 51 — read-only composite vCPU state verification

The current bounded slice adds one read-only verification boundary over an existing owned `VcpuStateSnapshot`, composing the existing composite capture and comparison paths without performing restore, repair, retry, or any other vCPU write.

Correctness contract:

- `Vcpu::verify_state_snapshot(reference)` performs exactly one existing composite state capture using only `reference.msrs().policy()` as the MSR access authority; callers do not supply or widen a separate verification policy;
- capture retains the existing canonical general-register → special-register → policy-bound MSR order and its existing short-circuit error behavior;
- if capture fails, the existing error propagates unchanged and no comparison result is fabricated;
- after a successful capture, verification delegates only to the existing pure `VcpuStateSnapshot::compare()` contract and returns its component-preserving `VcpuStateSnapshotComparison`;
- an exact current state returns an exact comparison, while any mismatch is returned as comparison data rather than an error;
- verification performs no restore, register write, special-register write, MSR write, retry, repair, rollback, or second capture;
- focused KVM-aware regression verifies both exact and mismatching states and proves that mismatch verification leaves the already-changed vCPU state unchanged;
- this read-only verification boundary does not make the underlying multi-ioctl composite capture atomic or quiesced and does not add migration compatibility, guest-memory/device capture, named CPU models, `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, SMP, long-mode/Linux boot, or resumable execution.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 51 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, architecture-documentation, CPU-model, or state-model work. Do not infer migration support, atomic/quiesced snapshots, guest-memory/device capture, `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, long-mode boot, SMP, resumable execution, or another CLI fixture automatically from the existence of read-only composite state verification.
