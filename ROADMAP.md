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
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion diagnostics, and unhandled-exit diagnostics, plus the minimal bidirectional debug port-I/O device.

## Phase 47 — user-facing capability documentation reconciliation

The current bounded slice reconciles the public README with the exact Phase 46 implementation state after the CPUID/MSR/state-snapshot and execution-diagnostics work outgrew the older user-facing capability summary. It intentionally changes no production behavior.

Correctness contract:

- the README must no longer claim that guest MSR policy or vCPU snapshots are unimplemented when both are already integrated;
- the README must describe configured CPUID application/read-back/proof, host/guest MSR modeling, general/special/composite vCPU state capture/compare/restore/restore-and-verify, typed HLT and legacy `KVM_EXIT_SHUTDOWN`, and ordered completed-exit diagnostics without overstating broader VMM capability;
- state snapshots must be described as owned CPU/MSR state boundaries rather than whole-VM, guest-memory, device-state, checkpoint, migration, or atomic/quiesced snapshots;
- composite restore must remain explicitly documented as bounded and non-transactional, including the absence of rollback after earlier component writes complete;
- exit-budget exhaustion must continue to preserve the pending-I/O completion caveat, and successful/budget-exhausted/unhandled paths must accurately describe their ordered completed-exit diagnostics;
- limitations must still explicitly exclude MMIO, multiple device families, interrupts/in-kernel interrupt-controller support, arbitrary CPU models, virtio, SMP, ELF loading, long-mode/Linux boot, migration orchestration, guest-memory/device snapshots, resumable execution, rollback, and `KVM_EXIT_SYSTEM_EVENT` payload policy;
- the README must point readers to this file as the authoritative current bounded implementation state and next-slice selector;
- this slice does not change Rust production code, KVM ABI handling, tests, CLI semantics, execution policy, state-model semantics, or safety assumptions; the normal full CI remains the regression gate for accidental repository breakage.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 47 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, architecture-documentation, or state-model work. In particular, do not infer that `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, long-mode boot, SMP, migration, or resumable execution are automatically next merely because the public README now accurately reflects the accumulated implementation.
