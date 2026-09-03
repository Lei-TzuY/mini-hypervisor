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
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_FAIL_ENTRY`, base `KVM_EXIT_INTERNAL_ERROR`, and `KVM_EXIT_SYSTEM_EVENT` diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, fail-entry, internal-error, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access;
- public README, architecture, and safety documentation synchronized through the Phase 59 documentation pass with the integrated Phase 58 internal-error base-diagnostic boundary.

## Phase 59 — internal-error architecture and safety documentation synchronization

The current bounded slice reconciles `ARCHITECTURE.md` and `docs/safety-assumptions.md` with the already integrated Phase 58 typed `KVM_EXIT_INTERNAL_ERROR` base diagnostic. It changes no Rust source, tests, KVM ABI behavior, execution policy, capability requirements, state mutation, or guest lifecycle semantics.

Correctness contract:

- the architecture map and execution narrative classify reason `17` as typed `VcpuExit::InternalError` and include the isolated `src/vcpu/internal_error.rs` base payload boundary;
- the documented x86 `kvm_run` view begins at union offset 32, reads only the always-available `suberror: u32`, and contributes a 40-byte required prefix after alignment;
- architecture and safety documentation state that `VcpuInternalError` owns only copied `suberror` state and no raw pointer or borrowed internal-error payload crosses into dispatch;
- dispatch documentation preserves the structured `VmExitError::InternalError` boundary and the deliberate absence of `KVM_GET_REGS` or another secondary vCPU ioctl that could obscure the completed-exit diagnostic;
- execution documentation preserves ordered completed-exit tracing with raw reason `17` exactly once at the trace tail after a successful `KVM_RUN`;
- the documents explicitly state that capability-dependent internal-error `ndata`/`data[16]` are not read, validated, sliced, copied, or exposed, and that this base diagnostic neither requires nor implies `KVM_CAP_INTERNAL_ERROR_DATA`;
- the documents do not invent internal-error retry, emulation recovery, replacement execution, lifecycle action, architecture-specific `suberror` policy, resumable execution, MMIO, interrupts, SMP, long-mode/Linux boot, migration, or guest-memory/device snapshot semantics;
- this slice changes documentation only; no production source, test source, workflow, or runtime behavior is modified, and repository CI remains the unchanged Format/Clippy/Test gate.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 59 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer `KVM_CAP_INTERNAL_ERROR_DATA` support, optional internal-error payload decoding, internal-error recovery/retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this documentation synchronization pass.
