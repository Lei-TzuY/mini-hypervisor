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
- composite vCPU state snapshots that own the existing general-register, special-register, and policy-bound MSR snapshots together, with pure component-preserving comparison, bounded non-transactional restore, and restore-and-verify;
- centralized VM-exit dispatch, bounded execution budgets, and the minimal bidirectional debug port-I/O device.

## Phase 42 — composite vCPU state snapshot restore-and-verify

The current bounded slice adds `Vcpu::restore_and_verify_state_snapshot()` by composing the existing composite restore, composite capture, and pure composite comparison boundaries.

Correctness contract:

- verification accepts only an already-owned `VcpuStateSnapshot`; it introduces no raw KVM representation, new state encoding, or new error taxonomy;
- verification first invokes the existing `restore_state_snapshot()` exactly once, preserving its dependency-aware special-register, general-register, then MSR restore order and non-transactional failure semantics;
- any restore failure prevents recapture and propagates unchanged, including existing partial-MSR-write diagnostics; no retry, rollback, or repair is attempted;
- only after restore succeeds, verification performs one `capture_state_snapshot()` using the MSR access policy already owned by the reference snapshot rather than caller-supplied or widened authority;
- any recapture failure propagates unchanged and prevents comparison; no second capture or rollback is attempted;
- successful restore and recapture are compared only through the existing pure `VcpuStateSnapshot::compare()` boundary, preserving typed component comparison and mismatch semantics;
- a non-exact comparison is returned as data rather than converted into an error, and does not trigger retry, repair, or a second restore;
- pure regressions lock restore-before-capture sequencing, restore/capture failure short-circuit behavior, and mismatch-report-only behavior; a KVM-aware regression requires a changed real-mode vCPU to restore and verify as an exact composite match when KVM is available;
- successful exact verification does not turn the underlying sequential capture or non-transactional restore into an atomic/quiesced architectural state operation;
- this slice does not add rollback, migration compatibility, guest-memory/device capture, atomic/quiesced snapshot semantics, long-mode boot, interrupts, MMIO, SMP, or device expansion.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 42 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further execution, architecture-documentation, or state-model work. Do not infer that migration orchestration, guest-memory/device capture, long-mode boot, interrupts, MMIO, SMP, or device expansion is automatically next merely because composite capture, comparison, restore, and restore-and-verify now exist.
