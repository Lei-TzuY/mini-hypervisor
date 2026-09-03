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
- public README, architecture, and safety documentation synchronized through the Phase 57 documentation pass with the integrated Phase 56 state and Phase 55 fail-entry contracts.

## Phase 57 — architecture and safety documentation synchronization

The current bounded slice reconciles the architecture and safety documentation with the already integrated Phase 55 typed fail-entry diagnostics and Phase 56 read-only component snapshot verification. It changes no Rust source, tests, KVM ABI behavior, execution policy, state mutation, or guest lifecycle semantics.

Correctness contract:

- the architecture map and execution narrative include typed `KVM_EXIT_FAIL_ENTRY` payload extraction, structured `EntryFailure` dispatch, and ordered completed-exit trace preservation without implying retry or recovery behavior;
- fail-entry documentation preserves the tested fixed payload boundary, owned `hardware_entry_failure_reason`/`cpu` diagnostics, and the deliberate absence of a secondary `KVM_GET_REGS` or other vCPU ioctl during dispatch;
- general-register, special-register, and guest-MSR documentation includes the Phase 56 read-only snapshot-bound verification APIs as exactly one fresh capture followed by the existing pure comparison;
- MSR verification remains bound to the reference snapshot's own `GuestMsrAccessPolicy`, while all component verification paths remain free of setters, restore, retry, repair, or rollback;
- the safety boundary includes the fail-entry `kvm_run` prefix in mapping-size reasoning and makes owned fail-entry/system-event payload lifetime semantics explicit;
- the documents continue to state that whole-VM/guest-memory/device snapshots, atomic/quiesced capture, migration, resumable execution, fail-entry retry/CPU-placement/recovery policy, internal-error capability plumbing, MMIO, interrupts, SMP, long-mode/Linux boot, and system-event lifecycle policy are not implemented;
- this slice changes documentation only; no production source, test source, workflow, or runtime behavior is modified, and repository CI remains the unchanged Format/Clippy/Test gate.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 57 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer internal-error capability plumbing, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this documentation synchronization pass.
