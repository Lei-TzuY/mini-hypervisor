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
- public README synchronized through the Phase 62 optional internal-error-data decoding boundary; architecture and safety documentation remain synchronized through the Phase 61 documentation pass.

## Phase 62 — capability-gated internal-error optional payload decoding

The current bounded slice consumes `KVM_EXIT_INTERNAL_ERROR` optional `ndata`/`data[16]` only on hosts that positively report `KVM_CAP_INTERNAL_ERROR_DATA`, while preserving the existing suberror-only behavior on hosts without that optional capability. It extends diagnostics only and does not add recovery or lifecycle policy.

Correctness contract:

- `KVM_CAP_INTERNAL_ERROR_DATA` remains optional and is not added to the five required KVM extensions;
- the already observed optional support boolean is propagated from `KvmBackend` through `Vm` into each created `Vcpu` without changing CPUID, memory, state, or execution setup;
- every `KVM_EXIT_INTERNAL_ERROR` continues to expose the always-available raw `suberror` as owned typed state;
- when optional data support is absent or non-positive, the decoder forms only the existing base view, does not read `ndata` or `data[16]`, and `VcpuInternalError::data()` returns `None`;
- when optional data support is positive, the decoder may form the fixed full x86 internal-error payload view only after the vCPU has inherited that host fact;
- capability-enabled decoding treats kernel `ndata` as untrusted metadata, requires `ndata <= 16` before any slice is formed, and copies only the declared words into owned Rust state;
- capability-enabled `ndata == 0` is distinguishable from capability absence: it produces available-but-empty optional data rather than `None`;
- a malformed capability-enabled `ndata` becomes structured `InvalidInternalErrorDataCount` diagnostics retaining vCPU id, raw `suberror`, reported `ndata`, fixed capacity, and the ordered completed-exit trace;
- normal structured `InternalError` diagnostics retain the raw `suberror`, optional owned data when available, and the complete ordered completed-exit trace without issuing a secondary register-read ioctl;
- no raw pointer or borrowed optional internal-error payload crosses into dispatch or execution diagnostics;
- no suberror-specific interpretation, emulation recovery, retry, replacement execution, lifecycle action, new KVM requirement, MMIO, interrupts, SMP, long-mode/Linux boot, migration, resumable execution, or guest-memory/device snapshot behavior is introduced;
- focused pure regressions lock base-vs-capability-enabled decoding, zero/full/malformed data counts, owned dispatch/trace preservation, and the public optional-data accessor, while the environment-sensitive KVM regression confirms the backend capability observation is propagated to created vCPUs when `/dev/kvm` is usable.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 62 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer internal-error suberror-specific recovery/retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this optional diagnostic decoding boundary.
