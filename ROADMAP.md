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
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_SYSTEM_EVENT` classification and owned payload extraction with structured unsupported diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access;
- public README, architecture, and safety documentation synchronized with the integrated CPU/MSR/state lifecycle, ordered execution traces, and typed system-event diagnostic boundaries.

## Phase 54 — documentation contract synchronization

The current bounded documentation slice reconciles public capability, architecture, and safety descriptions with the already-integrated implementation through Phase 53. It changes no Rust production behavior, KVM interaction, execution semantics, state mutation, or CLI command behavior.

Correctness contract:

- `README.md` must describe `KVM_CAP_GET_MSR_FEATURES`, the existing general/special/composite vCPU state comparison/verification/restore boundaries, typed `KVM_EXIT_SYSTEM_EVENT` payload extraction, 16-word `ndata` validation, structured unsupported-system-event diagnostics, and ordered completed-exit traces without claiming system-event lifecycle handling;
- `ARCHITECTURE.md` must no longer present already-completed general-register comparison/restore work as future work, must include the current special-register/composite-state lifecycle and CPU-model composition boundaries, and must describe legacy shutdown versus system-event dispatch semantics distinctly;
- `docs/safety-assumptions.md` must treat system-event type/count/data as untrusted shared-memory metadata, document the current minimum `kvm_run` prefix safety requirement and `ndata <= 16` check before slicing, and accurately describe owned execution traces and current state restore/verify capabilities;
- all three documents must preserve the existing limits: no MMIO, interrupts, in-kernel interrupt controller model, system-event reset/reboot/crash policy, SMP, long-mode/Linux boot, migration protocol, whole-VM or guest-memory/device snapshots, resumable execution, atomic/quiesced snapshot guarantee, or rollback;
- the historical `Next architectural milestone` text in `ARCHITECTURE.md` is replaced with a pointer to this authoritative roadmap so completed work cannot remain falsely preselected by stale architecture prose;
- documentation-only changes add no brittle string regression; validation is exact diff review plus the unchanged repository Format, Clippy, and Test CI gates.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 54 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, or architecture work. Do not infer system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from documentation synchronization.
