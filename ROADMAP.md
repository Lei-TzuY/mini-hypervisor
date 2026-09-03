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
- centralized VM-exit dispatch, bounded execution budgets, and the minimal bidirectional debug port-I/O device.

## Phase 38 — vCPU special-register restore-and-verify

The current bounded slice adds `Vcpu::restore_and_verify_special_register_snapshot()` as the verification layer above the existing owned special-register restore, capture, and pure comparison boundaries.

Correctness contract:

- verification accepts only an already-owned `VcpuSpecialRegisterSnapshot` and introduces no new state representation or public raw-KVM constructor;
- it delegates first to the existing `restore_special_register_snapshot()` path, so special-register encoding, UAPI padding zeroing, `KVM_SET_SREGS`, and write-error semantics remain owned by the Phase 37 boundary;
- only after restore succeeds does it perform one `capture_special_register_snapshot()` readback;
- a restore failure propagates unchanged and prevents readback; a readback failure after a successful restore propagates unchanged without retry or rollback;
- successful readback is compared through the existing pure `VcpuSpecialRegisterSnapshot::compare()` contract and returns its owned `VcpuSpecialRegisterSnapshotComparison` unchanged;
- a semantic mismatch is therefore a normal comparison result rather than a restore error, automatic repair request, or retry trigger;
- the focused KVM-aware regression requires a captured real-mode snapshot to flow through the public restore-and-verify path and return an exact owned comparison when KVM is available;
- this slice does not add multi-state snapshot composition, migration orchestration, rollback, long-mode boot, interrupts, MMIO, SMP, or device expansion.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 38 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further state composition, execution, or architecture-documentation work. Do not infer that multi-state snapshot composition, migration orchestration, long-mode boot, interrupts, MMIO, SMP, or device expansion is automatically next merely because general-register, MSR, and special-register snapshots now each have capture, comparison, restore, and restore-and-verify boundaries.
