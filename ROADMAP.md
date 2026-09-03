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
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_FAIL_ENTRY`, capability-gated `KVM_EXIT_INTERNAL_ERROR` optional diagnostics plus lossless typed classification of the four currently defined KVM internal-error suberrors, and `KVM_EXIT_SYSTEM_EVENT` diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, fail-entry, internal-error, malformed internal-error-data, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access;
- public README, architecture, and safety documentation synchronized through the Phase 63 documentation pass with the integrated Phase 62 capability-gated internal-error optional-payload boundary; the Phase 64 suberror-classification API may be reconciled in a later documentation pass.

## Phase 64 — typed internal-error suberror classification

The current bounded slice adds a policy-neutral typed classification for the stable KVM `KVM_EXIT_INTERNAL_ERROR` suberror values while preserving the existing raw `u32` diagnostic, optional-payload decoding, structured execution errors, and forward compatibility.

Correctness contract:

- the Linux KVM UAPI values `KVM_INTERNAL_ERROR_EMULATION = 1`, `KVM_INTERNAL_ERROR_SIMUL_EX = 2`, `KVM_INTERNAL_ERROR_DELIVERY_EV = 3`, and `KVM_INTERNAL_ERROR_UNEXPECTED_EXIT_REASON = 4` map to distinct public `VcpuInternalErrorSuberror` variants;
- every unrecognized `u32` maps to `VcpuInternalErrorSuberror::Unknown(raw)` so future kernel values are preserved rather than rejected or collapsed;
- `VcpuInternalErrorSuberror::from_raw(raw).raw()` round-trips every known and unknown raw value exactly;
- the existing `VcpuInternalError::suberror() -> u32` accessor remains unchanged, while `suberror_kind()` adds a read-only typed view over the same owned raw value;
- classification is identical on the base-only and capability-enabled optional-data decoder paths and does not change `VcpuInternalError::data()` semantics;
- existing `VmExitError::InternalError` and `InvalidInternalErrorDataCount` variants continue to retain the raw `suberror`, so structured execution diagnostics remain lossless and callers may classify that raw value without an error-shape change;
- no internal-error data word is reinterpreted according to suberror kind, and no emulation recovery, retry, replacement execution, lifecycle action, new KVM requirement, MMIO, interrupts, SMP, long-mode/Linux boot, migration, resumable execution, or guest-memory/device snapshot behavior is introduced;
- focused public regressions lock known mappings and unknown-value round trips, while decoder unit regressions confirm typed classification does not alter raw suberror or optional-data behavior.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 64 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer internal-error suberror/data-specific recovery or retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this diagnostic classification boundary.
