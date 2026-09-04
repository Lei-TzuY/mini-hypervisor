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
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_UNKNOWN` hardware diagnostics, typed `KVM_EXIT_EXCEPTION` diagnostics, typed `KVM_EXIT_FAIL_ENTRY`, capability-gated `KVM_EXIT_INTERNAL_ERROR` optional diagnostics plus lossless typed classification of the four currently defined KVM internal-error suberrors and a read-only interpretation of the stable `KVM_INTERNAL_ERROR_EMULATION` flags/instruction-byte overlay, and `KVM_EXIT_SYSTEM_EVENT` diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, KVM-unknown, exception, fail-entry, internal-error, malformed internal-error-data, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access;
- public README synchronized with the Phase 72 exception-diagnostic boundary; architecture and safety documentation remain synchronized through the Phase 71 documentation pass and may be reconciled in a later bounded documentation slice.

## Phase 72 — typed KVM exception diagnostics

The current bounded slice promotes Linux KVM exit reason `KVM_EXIT_EXCEPTION = 1` from the generic unsupported-reason path into a typed, owned, policy-neutral diagnostic while preserving all existing terminal, I/O, KVM-unknown, fail-entry, internal-error, system-event, budget, and state-model behavior.

Correctness contract:

- raw exit reason `1` maps to public `VcpuExit::Exception`, and `VcpuExit::Exception.reason()` round-trips exactly to `1`;
- the tested x86 `kvm_run` exception view begins at union offset `32`, owns only the fixed `exception: u32` and `error_code: u32` fields, has an 8-byte payload and 40-byte prefix, and therefore does not increase the existing common 168-byte mapping-size floor;
- `Vcpu::exception_exit()` rejects any non-exception current exit with structured `ExceptionPayloadUnavailable` rather than interpreting the wrong union member;
- successful decoding copies both fields into owned `VcpuException` state before the values cross into VM-exit policy;
- central dispatch converts the purpose-built payload directly into structured `VmExitError::Exception` and deliberately issues no `KVM_GET_REGS` or other secondary vCPU ioctl that could obscure the completed exception exit;
- the execution layer preserves the complete ordered completed-exit trace on exception diagnostics, with reason `1` appearing exactly once at the tail because the successful `KVM_RUN` is recorded before dispatch;
- exception vector and error-code values remain opaque diagnostics; no exception injection, reinjection, emulation, retry, replacement execution, recovery, lifecycle action, or architecture-specific interpretation is introduced;
- existing HLT, I/O, legacy shutdown, KVM-unknown, fail-entry, internal-error, system-event, generic unhandled, execution-budget, CPU/MSR/state, memory, and CLI semantics remain unchanged;
- focused public and pure regressions lock raw classification, x86 layout, exact payload ownership, dispatch behavior without register context, and complete-trace replacement.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 72 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer exception injection/recovery, emulation recovery or instruction emulation, arbitrary internal-error debug-data interpretation, KVM-unknown hardware-reason interpretation/recovery, internal-error suberror-specific retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this diagnostic boundary.
