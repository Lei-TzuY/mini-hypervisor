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
- public README synchronized through the Phase 60 optional-capability observation boundary; architecture and safety documentation remain synchronized through the Phase 59 documentation pass.

## Phase 60 — optional internal-error-data capability observation

The current bounded slice records `KVM_CAP_INTERNAL_ERROR_DATA` as optional host capability metadata without changing the required KVM host contract or consuming capability-dependent `KVM_EXIT_INTERNAL_ERROR` payload fields.

Correctness contract:

- Linux KVM capability ID `40` is queried through the existing `KVM_CHECK_EXTENSION` boundary when `KvmBackend` opens;
- the returned raw capability value is retained as a normal `Capability` entry inside the existing owned `HostCapabilities.extensions` snapshot;
- `HostCapabilities::internal_error_data_capability` exposes the recorded observation read-only when present, including a raw value of `0`;
- `HostCapabilities::supports_internal_error_data` is true exactly when the recorded capability value is greater than zero;
- `HostCapabilities::validate` continues to require only the existing five `REQUIRED_EXTENSIONS`; an absent manually constructed optional observation or a recorded value of `0` does not invalidate otherwise valid host capabilities;
- existing required-extension failure semantics, KVM API-version validation, vCPU mmap-size validation, CPUID/MSR discovery, VM/vCPU construction, and execution behavior remain unchanged;
- the base `KVM_EXIT_INTERNAL_ERROR` decoder continues to read and own only always-available `suberror`; it does not read, validate, slice, copy, or expose capability-dependent `ndata` or `data[16]` even when the optional capability is reported available;
- no optional-payload decoder, emulation-recovery policy, retry, replacement execution, lifecycle action, new KVM requirement, MMIO, interrupts, SMP, long-mode/Linux boot, migration, resumable execution, or guest-memory/device snapshot behavior is introduced;
- focused pure regression coverage locks absent/zero/present optional capability semantics, while the environment-sensitive KVM regression confirms a real backend records the ID-40 observation whenever `/dev/kvm` is usable.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 60 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer optional internal-error `ndata`/`data[16]` decoding, internal-error recovery/retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this optional capability observation boundary.
