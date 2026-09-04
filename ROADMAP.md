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
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_UNKNOWN` hardware diagnostics, typed fixed-payload `KVM_EXIT_EXCEPTION` diagnostics, typed `KVM_EXIT_FAIL_ENTRY`, capability-gated `KVM_EXIT_INTERNAL_ERROR` optional diagnostics plus lossless typed classification of the four currently defined KVM internal-error suberrors and a read-only interpretation of the stable `KVM_INTERNAL_ERROR_EMULATION` flags/instruction-byte overlay, and `KVM_EXIT_SYSTEM_EVENT` diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, KVM-unknown, exception, fail-entry, internal-error, malformed internal-error-data, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access;
- public README, architecture, and safety documentation synchronized through the integrated Phase 69 read-only emulation-failure metadata accessor boundary; architecture and safety may lag the new Phase 72 exception diagnostic boundary by one documentation pass.

## Phase 72 — typed KVM exception diagnostics

The current bounded slice adds a policy-neutral typed boundary for Linux `KVM_EXIT_EXCEPTION` reason `1`, preserving the fixed exception-vector/error-code payload as owned diagnostics and extending the existing ordered completed-exit trace contract without introducing exception reinjection, recovery, or lifecycle behavior.

Correctness contract:

- raw reason `KVM_EXIT_EXCEPTION = 1` maps to public `VcpuExit::Exception`, and `VcpuExit::Exception.reason()` round-trips exactly to `1`;
- the tested x86 `kvm_run` exception view begins at union offset `32`, owns exactly the fixed `{ exception: u32, error_code: u32 }` payload, and requires only the fixed `40`-byte prefix already below the current common mapping-size floor;
- `Vcpu::exception_exit()` rejects any current exit reason other than `1` with structured `ExceptionPayloadUnavailable` diagnostics rather than reading the wrong union member;
- successful payload extraction copies both raw fields exactly into owned `VcpuExceptionExit` state and retains no pointer or borrow into the shared `kvm_run` mapping;
- central dispatch converts the typed exception payload directly into structured `VmExitError::Exception` diagnostics and deliberately issues no secondary `KVM_GET_REGS` or other vCPU ioctl that could obscure the completed exit;
- the execution loop replaces the local one-reason trace with the complete ordered completed-exit trace while keeping reason `1` exactly once at the tail;
- no exception-vector interpretation, guest exception reinjection, retry, replacement execution, additional `KVM_RUN`, lifecycle action, new KVM requirement, MMIO, interrupts, SMP, long-mode/Linux boot, migration, resumable execution, or guest-memory/device snapshot behavior is introduced;
- focused public and pure regressions lock raw classification, UAPI layout, exact payload ownership, dispatch context, and complete trace replacement.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 72 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer exception reinjection/recovery, emulation recovery or instruction emulation, arbitrary internal-error debug-data interpretation, KVM-unknown hardware-reason interpretation/recovery, internal-error suberror-specific retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this diagnostic boundary.
