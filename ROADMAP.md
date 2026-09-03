# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host required-capability validation plus optional `KVM_CAP_INTERNAL_ERROR_DATA` observation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, pure policy comparison, and a deterministic CLI guest-proof fixture;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- composite CPU-model candidates that own the configured guest CPUID policy together with the immutable host MSR model candidate, including backend materialization, component-preserving pure comparison, and aggregate exactness;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, read-only snapshot-bound verification, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots, pure 18-field reference-to-observed comparison, read-only snapshot-bound verification, snapshot-bound restore, and restore-and-verify;
- owned vCPU special-register snapshots covering segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM UAPI padding, plus pure deterministic semantic-field comparison, read-only snapshot-bound verification, snapshot-bound restore, and restore-and-verify;
- composite vCPU state snapshots that own the existing general-register, special-register, and policy-bound MSR snapshots together, with pure component-preserving comparison, read-only snapshot-bound verification, bounded non-transactional restore, restore-and-verify, and a deterministic public/CLI round-trip fixture;
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_FAIL_ENTRY`, base `KVM_EXIT_INTERNAL_ERROR`, and `KVM_EXIT_SYSTEM_EVENT` diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, fail-entry, internal-error, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access;
- public README, architecture, and safety documentation synchronized through the Phase 61 documentation pass with the integrated Phase 60 optional internal-error-data capability-observation boundary.

## Phase 61 — optional internal-error capability architecture and safety synchronization

The current bounded slice reconciles `ARCHITECTURE.md` and `docs/safety-assumptions.md` with the already integrated Phase 60 optional `KVM_CAP_INTERNAL_ERROR_DATA` observation. It changes no Rust source, test source, KVM ABI behavior, execution policy, required-capability contract, state mutation, or guest lifecycle semantics.

Correctness contract:

- architecture documentation distinguishes the five required KVM extensions from the separately observed optional `KVM_CAP_INTERNAL_ERROR_DATA` capability;
- the optional capability observation is recorded as ordinary owned `HostCapabilities` metadata and a value of `0` remains valid for the current host contract;
- documentation states that `internal_error_data_capability()` exposes the recorded observation when present and `supports_internal_error_data()` reflects only whether its raw value is greater than zero;
- a positive optional observation does not enlarge the current 40-byte typed internal-error base view and does not authorize reading, validating, slicing, copying, or exposing capability-dependent `ndata` or `data[16]`;
- the typed `VcpuInternalError` boundary continues to own only the always-available raw `suberror`, and dispatch/execution behavior remain unchanged;
- the documents do not imply optional-payload support, emulation recovery, retry, replacement execution, lifecycle action, MMIO, interrupts, SMP, long-mode/Linux boot, migration, resumable execution, or guest-memory/device snapshot semantics;
- this slice changes documentation only; repository Format/Clippy/Test CI remains the unchanged integration gate.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 61 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer optional internal-error `ndata`/`data[16]` decoding, internal-error recovery/retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this documentation synchronization pass.
