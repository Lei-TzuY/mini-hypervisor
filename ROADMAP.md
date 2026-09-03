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
- public README, architecture, and safety documentation synchronized through the Phase 65 documentation pass with the integrated Phase 64 typed internal-error suberror-classification boundary.

## Phase 65 — internal-error suberror classification documentation synchronization

The current bounded slice reconciles `README.md`, `ARCHITECTURE.md`, and `docs/safety-assumptions.md` with the already integrated Phase 64 policy-neutral `VcpuInternalErrorSuberror` API. It changes no Rust source, test source, KVM ABI behavior, execution policy, required-capability contract, state mutation, or guest lifecycle semantics.

Correctness contract:

- documentation records the Linux KVM internal-error suberror values 1 through 4 as `Emulation`, `SimultaneousExceptions`, `DeliveryEvent`, and `UnexpectedExitReason` respectively;
- every other raw `u32` remains losslessly represented as `VcpuInternalErrorSuberror::Unknown(raw)`, and typed values round-trip to their exact raw values through `raw()`;
- `VcpuInternalError::suberror() -> u32` remains documented as the unchanged raw source of truth, while `suberror_kind()` is only a read-only pure classification of that already-owned scalar;
- base-only and capability-enabled optional-data decoding are documented as deriving identical typed classification from the same copied raw suberror;
- existing `VmExitError::InternalError` and `InvalidInternalErrorDataCount` diagnostics remain raw-suberror-bearing shapes rather than being rewritten around the enum, preserving lossless structured diagnostics and compatibility;
- typed classification does not alter `KVM_CAP_INTERNAL_ERROR_DATA` optionality, mapping-size requirements, `ndata <= 16` validation, `None` versus available-empty optional-data semantics, completed-exit traces, or any unsafe shared-memory access boundary;
- documentation does not interpret optional data words according to suberror kind and does not imply emulation recovery, retry, replacement execution, lifecycle action, a new required KVM capability, MMIO, interrupts, SMP, long-mode/Linux boot, migration, resumable execution, or guest-memory/device snapshot behavior;
- this slice changes documentation only; repository Format/Clippy/Test CI remains the unchanged integration gate.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 65 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer internal-error suberror/data-specific recovery or retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this documentation synchronization pass.
