# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host capability validation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, and pure policy comparison;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots, pure 18-field reference-to-observed comparison, snapshot-bound restore, and restore-and-verify;
- owned vCPU special-register snapshots covering segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM UAPI padding, plus pure deterministic semantic-field comparison and snapshot-bound restore;
- centralized VM-exit dispatch, bounded execution budgets, and the minimal bidirectional debug port-I/O device.

## Phase 37 — vCPU special-register snapshot restore

The current bounded slice adds a snapshot-bound `Vcpu::restore_special_register_snapshot()` path that reconstructs the x86 KVM special-register UAPI from owned semantic state and performs one existing `KVM_SET_SREGS` write.

Correctness contract:

- restore accepts only an already-owned `VcpuSpecialRegisterSnapshot`; it does not add public constructors for arbitrary special-register state;
- CS/DS/ES/FS/GS/SS/TR/LDT semantic fields are copied exactly into fresh `KvmSegment` values, while the UAPI-only padding byte is always zeroed;
- GDT/IDT base and limit are copied exactly into fresh `KvmDtable` values, while all three UAPI-only padding words are always zeroed;
- CR0/CR2/CR3/CR4/CR8, EFER, APIC base, and all four interrupt-bitmap words are copied exactly without normalization, masking, synthesis, or interpretation;
- restore performs exactly one `KVM_SET_SREGS` attempt after pure encoding and reports failure through the existing named vCPU-operation error boundary;
- pure regressions lock semantic segment/descriptor-table encoding and deterministic padding zeroing;
- a KVM-aware regression captures explicitly initialized real-mode special-register state, restores that owned snapshot, recaptures it, and requires the existing pure comparison contract to report an exact match when KVM is available;
- this slice does not add a dedicated restore-and-verify convenience API, rollback, multi-state composition, migration orchestration, long-mode boot, interrupts, MMIO, SMP, or device expansion.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 37 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further state verification, composition, or execution work. Do not infer that special-register restore-and-verify, multi-state snapshot composition, migration orchestration, long-mode boot, interrupts, MMIO, SMP, or device expansion is automatically next merely because snapshot-bound restore now exists.
