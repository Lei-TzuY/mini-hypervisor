# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host capability validation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, and pure policy comparison;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots, pure 18-field reference-to-observed comparison, snapshot-bound restore, and restore-and-verify;
- owned vCPU special-register snapshots covering segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM UAPI padding, plus pure deterministic semantic-field comparison;
- centralized VM-exit dispatch, bounded execution budgets, and the minimal bidirectional debug port-I/O device.

## Phase 36 — vCPU special-register snapshot comparison

The current bounded slice adds a pure comparison contract for two `VcpuSpecialRegisterSnapshot` values plus typed mismatch identity for every captured semantic field.

Correctness contract:

- comparison performs no KVM ioctl and never mutates either source snapshot;
- segment identity is explicit for CS/DS/ES/FS/GS/SS/TR/LDT, with stable child-field identity for base, limit, selector, type, present, DPL, DB, S, L, G, AVL, and unusable;
- descriptor-table identity is explicit for GDT/IDT base and limit;
- CR0/CR2/CR3/CR4/CR8, EFER, APIC base, and all four interrupt-bitmap words have named typed mismatch identity;
- every mismatch retains the exact reference and observed semantic values as `u64` without normalization, masking, synthesis, or interpretation;
- mismatch ordering is canonical and deterministic: segment register order, then segment child-field order, then GDT/IDT, then scalar special registers, then interrupt-bitmap word order;
- the comparison owns complete copies of both source snapshots so diagnostics remain valid after the original values and vCPU are gone;
- focused pure regressions lock exact-match behavior, nested one-field diagnostics, canonical multi-field ordering, and owned/cloneable comparison state, while the KVM-aware regression proves a real captured special-register snapshot can flow through the public comparison boundary;
- this slice does not add `KVM_SET_SREGS`, special-register restore or restore-and-verify, multi-state composition, migration orchestration, long-mode boot, interrupts, MMIO, SMP, or device expansion.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 36 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further state restoration, composition, or execution work. Do not infer that special-register restore, multi-state snapshot composition, migration orchestration, long-mode boot, interrupts, MMIO, SMP, or device expansion is automatically next merely because comparison now exists.
