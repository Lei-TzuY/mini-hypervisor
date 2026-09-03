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
- public README synchronized through the Phase 58 internal-error base-diagnostic boundary; architecture and safety documentation remain synchronized through the Phase 57 documentation pass.

## Phase 58 — typed KVM internal-error base diagnostics

The current bounded slice classifies `KVM_EXIT_INTERNAL_ERROR` as a typed exit and preserves its always-available `suberror` field as owned structured diagnostic state. It deliberately does not introduce optional-capability plumbing or consume capability-dependent internal-error data.

Correctness contract:

- raw exit reason `17` maps to `VcpuExit::InternalError`, and `VcpuExit::InternalError.reason()` round-trips to `17`;
- `Vcpu::internal_error` is available only when the current shared `kvm_run` exit reason is `KVM_EXIT_INTERNAL_ERROR`; any other reason returns a structured payload-unavailable error;
- the tested x86 base view reads only the union-offset `suberror: u32` field and copies it into owned `VcpuInternalError` state;
- the decoder does not read, validate, copy, or expose `ndata` or `data[16]`, and therefore does not require or imply `KVM_CAP_INTERNAL_ERROR_DATA`;
- dispatch converts the typed exit into `VmExitError::InternalError` containing vCPU id, raw suberror, and a local one-element reason trace without issuing `KVM_GET_REGS` or another secondary vCPU ioctl;
- the common execution loop replaces that local trace with the complete ordered completed-exit trace, preserving reason `17` exactly once as the final completed exit;
- no additional `KVM_RUN`, retry, emulation recovery, replacement execution, architecture-specific suberror policy, optional internal-error data decoding, CPU placement, lifecycle action, or resumable execution is introduced;
- existing HLT, I/O, legacy shutdown, fail-entry, system-event, unknown-exit, budget, state-snapshot, CPU/MSR policy, memory, and CLI semantics remain unchanged;
- focused public and pure regressions lock typed classification, raw round-trip, x86 union offset/base-prefix size, exact suberror copying, dispatch ownership, and execution-trace replacement.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 58 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer `KVM_CAP_INTERNAL_ERROR_DATA` support, optional internal-error payload decoding, internal-error recovery/retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this base diagnostic boundary.
