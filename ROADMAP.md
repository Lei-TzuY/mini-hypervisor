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
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_UNKNOWN` hardware diagnostics, typed `KVM_EXIT_FAIL_ENTRY`, capability-gated `KVM_EXIT_INTERNAL_ERROR` optional diagnostics plus lossless typed classification of the four currently defined KVM internal-error suberrors, and `KVM_EXIT_SYSTEM_EVENT` diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, KVM-unknown, fail-entry, internal-error, malformed internal-error-data, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access;
- public README, architecture, and safety documentation synchronized through the Phase 65 documentation pass with the integrated Phase 64 typed internal-error suberror-classification boundary; the Phase 66 KVM-unknown diagnostic boundary may be reconciled in a later documentation pass.

## Phase 66 — typed KVM unknown-hardware exit diagnostics

The current bounded slice separates Linux KVM's explicit `KVM_EXIT_UNKNOWN = 0` exit from this project's generic unsupported-raw-reason path and preserves the fixed x86 `hardware_exit_reason` payload as owned structured diagnostics without introducing hardware-specific recovery policy.

Correctness contract:

- raw exit reason `0` maps to the distinct public `VcpuExit::KvmUnknown` variant and round-trips back to `0`, while other unsupported raw reasons continue to map to `VcpuExit::Unhandled { reason }` unchanged;
- the tested x86 `kvm_run` unknown-exit view begins at union offset 32, contains exactly one `u64 hardware_exit_reason`, and therefore requires a 40-byte prefix already covered by the current common mapping-size floor;
- `Vcpu::kvm_unknown_exit()` requires the current raw exit reason to be `KVM_EXIT_UNKNOWN` before reading the payload and copies the hardware reason into owned `VcpuKvmUnknownExit` state;
- using the payload accessor for another exit reason returns structured `KvmUnknownPayloadUnavailable` rather than reading the wrong union member;
- central VM-exit dispatch turns `VcpuExit::KvmUnknown` into `VmExitError::KvmUnknownExit` retaining vCPU id, raw hardware exit reason, and a local one-element reason trace;
- KVM-unknown dispatch deliberately does not issue `KVM_GET_REGS` or another secondary vCPU ioctl that could replace the original purpose-built diagnostic with a new host error;
- the common execution loop replaces the local reason trace with the complete ordered completed-exit trace, preserving every prior completed exit and reason `0` exactly once at the tail;
- existing generic `Unhandled` behavior, including RIP/RFLAGS register context for other unsupported raw reasons, remains unchanged;
- `hardware_exit_reason` is retained as opaque KVM/hardware diagnostic metadata and is not translated into SGX/VMX policy, retry, recovery, replacement execution, lifecycle action, a new KVM requirement, MMIO, interrupts, SMP, long-mode/Linux boot, migration, resumable execution, or guest-memory/device snapshot behavior;
- a focused public regression locks reason-0 typed classification without collapsing other unsupported reasons, while pure decoder/dispatch/execution regressions lock the fixed layout, owned hardware reason, local diagnostic shape, and complete-trace replacement.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 66 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer KVM-unknown hardware-reason interpretation/recovery, internal-error suberror/data-specific recovery or retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this diagnostic boundary.
