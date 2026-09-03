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
- composite vCPU state snapshots that own the existing general-register, special-register, and policy-bound MSR snapshots together, with pure component-preserving comparison, bounded non-transactional restore, restore-and-verify, and a deterministic public/CLI round-trip fixture;
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion diagnostics, and unhandled-exit diagnostics, plus the minimal bidirectional debug port-I/O device.

## Phase 50 — backend CPU-model materialization and exactness

The current bounded slice makes the already-existing `CpuModelCandidate` composition directly materializable from `KvmBackend` and gives its composite comparison one authoritative exact-match predicate without changing host discovery, configured guest CPU policy, MSR stability classification, or any KVM state.

Correctness contract:

- `KvmBackend::cpu_model_candidate()` derives its result only from the backend's already-owned configured `GuestCpuPolicy` and `HostMsrFeatureValues::model_candidate()`; calling it issues no ioctl, creates no VM/vCPU, and mutates no backend state;
- the materialized candidate's guest-CPU-policy component must exactly equal `KvmBackend::cpu_policy()`;
- the materialized host-MSR-model component must retain the complete `KvmBackend::host_msr_feature_values()` observation as source provenance while continuing to compare only the existing `ModelImmutable` subset;
- the returned candidate is owned and remains valid independently of the backend lifetime;
- `CpuModelComparison::is_exact_match()` is true if and only if both the configured guest-CPUID comparison and immutable host-MSR-model comparison are exact;
- CPUID-only drift, MSR-model-only drift, or drift in both components must each make the composite exactness predicate false without flattening or replacing the existing component diagnostics;
- focused pure regressions lock the exact, CPUID-only-drift, MSR-only-drift, mixed-drift, and empty-candidate cases, while a KVM-aware regression verifies backend materialization and provenance when `/dev/kvm` is available;
- this slice does not add named/configurable CPU models, migration compatibility, guest-MSR lifecycle compatibility, cross-kernel portability guarantees, VM/vCPU mutation, state restore, `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, SMP, long-mode/Linux boot, or resumable execution.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 50 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, architecture-documentation, CPU-model, or state-model work. Do not infer migration support, a named CPU-model layer, `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, long-mode boot, SMP, resumable execution, or another CLI fixture automatically from the existence of a backend-materialized composite CPU-model candidate.
