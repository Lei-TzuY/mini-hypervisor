# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host required-capability validation plus optional `KVM_CAP_INTERNAL_ERROR_DATA` observation and capability-gated vCPU propagation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, pure policy comparison, and a deterministic CLI guest-proof fixture;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- composite CPU-model candidates that own the configured guest CPUID policy together with the immutable host MSR model candidate, including backend materialization, component-preserving pure comparison, and aggregate exactness;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, read-only snapshot-bound verification, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots, pure 18-field reference-to-observed comparison, read-only snapshot-bound verification, snapshot-bound restore, and restore-and-verify;
- owned vCPU special-register snapshots covering segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM UAPI padding, plus pure deterministic semantic-field comparison, read-only snapshot-bound verification, snapshot-bound restore, and restore-and-verify;
- composite vCPU state snapshots that own the existing general-register, special-register, and policy-bound MSR snapshots together, with pure component-preserving comparison, read-only snapshot-bound verification, bounded non-transactional restore, restore-and-verify, and a deterministic public/CLI round-trip fixture;
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_FAIL_ENTRY`, capability-gated `KVM_EXIT_INTERNAL_ERROR` optional diagnostics, and `KVM_EXIT_SYSTEM_EVENT` diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, fail-entry, internal-error, malformed internal-error-data, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access;
- public README, architecture, and safety documentation synchronized through the Phase 63 documentation pass with the integrated Phase 62 capability-gated internal-error optional-payload boundary.

## Phase 63 — internal-error optional payload architecture and safety synchronization

The current bounded slice reconciles `ARCHITECTURE.md` and `docs/safety-assumptions.md` with the already integrated Phase 62 capability-gated `KVM_EXIT_INTERNAL_ERROR` optional-payload diagnostics. It changes no Rust source, test source, KVM ABI behavior, execution policy, required-capability contract, state mutation, or guest lifecycle semantics.

Correctness contract:

- architecture and safety documentation continue to distinguish the five required KVM extensions from optional `KVM_CAP_INTERNAL_ERROR_DATA`; a missing or non-positive observation remains valid and preserves the base `suberror`-only decoder path;
- documentation records that the positive optional-support fact is propagated from `KvmBackend` through `Vm` into each created `Vcpu` without changing CPUID, memory, state, or execution setup;
- the base internal-error view remains the only view formed when optional support is absent, and `VcpuInternalError::data()` is documented as `None` on that path;
- only a vCPU that inherited positive optional support may form the fixed full x86 internal-error payload view containing `suberror`, `ndata`, and `data[16]`;
- kernel `ndata` is documented as untrusted metadata that must be `<= 16` before any Rust slice is formed, with only declared words copied into owned state;
- capability-enabled `ndata == 0` remains distinguishable from unavailable optional data as available-but-empty data;
- architecture/safety text documents structured `InvalidInternalErrorDataCount` diagnostics and preservation of the ordered completed-exit trace for malformed optional-data counts;
- ownership text reflects that typed internal-error state and execution diagnostics contain only copied Rust data and no pointer or borrow into `kvm_run`;
- documentation does not imply suberror/data-specific interpretation, emulation recovery, retry, replacement execution, lifecycle action, a new required capability, MMIO, interrupts, SMP, long-mode/Linux boot, migration, resumable execution, or guest-memory/device snapshot behavior;
- this slice changes documentation only; repository Format/Clippy/Test CI remains the unchanged integration gate.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 63 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer internal-error suberror/data-specific recovery or retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this documentation synchronization pass.
