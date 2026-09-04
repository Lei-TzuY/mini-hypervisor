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
- public README, architecture, and safety documentation synchronized with the integrated Phase 72 typed `KVM_EXIT_EXCEPTION` diagnostic boundary.

## Phase 73 — exception diagnostic architecture and safety synchronization

The current bounded slice reconciles `ARCHITECTURE.md` and `docs/safety-assumptions.md` with the already integrated Phase 72 typed `KVM_EXIT_EXCEPTION` diagnostic boundary. It changes no Rust source, test source, KVM ABI behavior, execution policy, required-capability contract, state mutation, or guest lifecycle semantics.

Correctness contract:

- architecture and safety documentation record raw exit reason `KVM_EXIT_EXCEPTION = 1` as public `VcpuExit::Exception` while preserving existing raw-reason round-trip semantics;
- the documented fixed x86 `kvm_run` exception view begins at union offset `32`, contains only `exception: u32` and `error_code: u32`, has an 8-byte payload and a 40-byte required prefix, and does not increase the existing 168-byte common mapping-size floor;
- `Vcpu::exception_exit()` is documented as rejecting any non-exception current exit with structured `ExceptionPayloadUnavailable` rather than inspecting the wrong union member;
- successful decode is documented as copying both fields into owned `VcpuException` state before higher-level policy sees them;
- central dispatch is documented as returning structured `VmExitError::Exception` directly from the purpose-built payload without `KVM_GET_REGS` or another secondary vCPU ioctl;
- the execution loop is documented as recording reason `1` before dispatch and preserving the complete ordered completed-exit trace on exception diagnostics, with the reason appearing exactly once at the trace tail;
- exception vector and error-code fields remain opaque diagnostic metadata; documentation does not infer exception injection or reinjection, instruction emulation, retry, replacement execution, recovery, lifecycle action, or architecture-specific policy;
- existing HLT, I/O, legacy shutdown, KVM-unknown, fail-entry, internal-error, system-event, generic unhandled, execution-budget, CPU/MSR/state, memory, and CLI semantics remain unchanged;
- the documentation update is additive and focused: no production, test, workflow, configuration, or runtime file is changed.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 73 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer exception injection/recovery, emulation recovery or instruction emulation, arbitrary internal-error debug-data interpretation, KVM-unknown hardware-reason interpretation/recovery, internal-error suberror-specific retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this documentation synchronization boundary.
