# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host capability validation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, pure policy comparison, and a deterministic CLI guest-proof fixture;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- composite CPU-model candidates that own the configured guest CPUID policy together with the immutable host MSR model candidate, including backend materialization, component-preserving pure comparison, and aggregate exactness;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, read-only snapshot-bound verification, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots, pure 18-field reference-to-observed comparison, read-only snapshot-bound verification, snapshot-bound restore, and restore-and-verify;
- owned vCPU special-register snapshots covering segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM UAPI padding, plus pure deterministic semantic-field comparison, read-only snapshot-bound verification, snapshot-bound restore, and restore-and-verify;
- composite vCPU state snapshots that own the existing general-register, special-register, and policy-bound MSR snapshots together, with pure component-preserving comparison, read-only snapshot-bound verification, bounded non-transactional restore, restore-and-verify, and a deterministic public/CLI round-trip fixture;
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_FAIL_ENTRY` and `KVM_EXIT_SYSTEM_EVENT` payload diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, fail-entry, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access;
- public README synchronized with the Phase 56 component-verification boundary; architecture and safety documentation remain synchronized through the Phase 54 documentation pass.

## Phase 56 — read-only component snapshot verification

The current bounded slice adds the same snapshot-bound read-only verification operation already available for composite vCPU state to the individual general-register, special-register, and guest-MSR snapshot boundaries. It changes no restore, execution, guest-memory, device, migration, or lifecycle semantics.

Correctness contract:

- `Vcpu::verify_register_snapshot` performs exactly one fresh general-register capture and returns the existing `VcpuRegisterSnapshotComparison` against the supplied reference snapshot;
- `Vcpu::verify_special_register_snapshot` performs exactly one fresh special-register capture and returns the existing `VcpuSpecialRegisterSnapshotComparison` against the supplied reference snapshot;
- `Vcpu::verify_msr_snapshot` performs exactly one fresh MSR snapshot capture using the reference snapshot's own bound `GuestMsrAccessPolicy`, then returns the existing `GuestMsrSnapshotComparison`;
- all three verification paths are read-only: they must not invoke `KVM_SET_REGS`, `KVM_SET_SREGS`, `KVM_SET_MSRS`, restore helpers, retry, repair, or rollback;
- an exact observation returns an exact existing typed comparison, while a mismatch is returned as comparison data and does not trigger mutation or automatic repair;
- capture errors propagate unchanged, without retry and without invoking comparison after the failed capture;
- the existing composite `verify_state_snapshot` behavior and capture ordering remain unchanged;
- a KVM-aware regression proves special-register and general-register mismatches remain present after verification, while a portability-safe empty-policy MSR verification remains unchanged before and after verification; only KVM-unavailable or permission-denied environments are skipped;
- this slice adds no whole-VM, guest-memory, device-state, migration, checkpoint, atomic/quiesced snapshot, resumable execution, retry, repair, or rollback semantics.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 56 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, or architecture work. Do not infer internal-error capability plumbing, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from component verification symmetry.
