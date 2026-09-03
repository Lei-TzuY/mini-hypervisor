# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host capability validation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, and pure policy comparison;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots, pure 18-field reference-to-observed comparison, snapshot-bound restore, and restore-and-verify;
- owned vCPU special-register snapshots covering segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM UAPI padding;
- centralized VM-exit dispatch, bounded execution budgets, and the minimal bidirectional debug port-I/O device.

## Phase 35 — vCPU special-register snapshot capture

The current bounded slice adds `Vcpu::capture_special_register_snapshot()` plus typed owned `VcpuSpecialRegisterSnapshot`, `VcpuSegmentState`, and `VcpuDescriptorTableState` representations.

Correctness contract:

- each capture performs exactly one existing `KVM_GET_SREGS` read and does not write guest state;
- the owned snapshot preserves CS/DS/ES/FS/GS/SS/TR/LDT semantic fields, GDT/IDT base and limit, CR0/CR2/CR3/CR4/CR8, EFER, APIC base, and all four interrupt-bitmap words;
- KVM segment and descriptor-table padding is deliberately absent from the public typed state;
- capture copies the kernel-reported semantic values without normalization, masking, synthesis, or interpretation;
- `KVM_GET_SREGS` failure remains a named vCPU operation failure through the existing error boundary;
- focused pure regressions lock complete semantic field copying and padding exclusion, while a KVM-aware regression proves the explicitly initialized real-mode segment bases/selectors and CR0 PE/PG state are observable through the snapshot;
- this slice does not compare, restore, restore-and-verify, compose, or migrate special-register state and does not modify CPUID, MSR, memory, device, or multi-vCPU lifecycle behavior.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 35 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further state-comparison, restoration, or composition work. Do not infer that special-register comparison or restore, multi-state snapshot composition, migration orchestration, long-mode boot, interrupts, MMIO, SMP, or device expansion is automatically next merely because special-register capture now exists.
