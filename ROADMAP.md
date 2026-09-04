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
- public README, architecture, and safety documentation synchronized with the integrated Phase 66 typed KVM-unknown diagnostic boundary.

## Phase 68 — KVM-unknown architecture and safety synchronization

The current bounded slice reconciles `ARCHITECTURE.md` and `docs/safety-assumptions.md` with the already integrated Phase 66 `KVM_EXIT_UNKNOWN` hardware-diagnostic boundary and the Phase 67 public README summary. It changes no Rust source, test source, KVM ABI behavior, execution policy, state mutation, capability requirement, or guest lifecycle semantics.

Correctness contract:

- architecture documentation distinguishes raw exit reason `0` as the typed `VcpuExit::KvmUnknown` path from generic unsupported raw reasons that remain `Unhandled { reason }`;
- the tested fixed 40-byte x86 KVM-unknown prefix and owned `VcpuKvmUnknownExit::hardware_exit_reason` boundary are documented without changing the existing 168-byte common mapping-size floor;
- `Vcpu::kvm_unknown_exit()` is documented as validating the current raw exit reason before reading the union member, with misuse represented by `KvmUnknownPayloadUnavailable` rather than an unchecked union read;
- central dispatch is documented as returning structured `VmExitError::KvmUnknownExit` without a secondary `KVM_GET_REGS` or other vCPU ioctl that could obscure the purpose-built hardware diagnostic;
- execution documentation preserves the complete ordered completed-exit trace with raw reason `0` appearing exactly once at the tail of a KVM-unknown diagnostic;
- ownership and safety documentation state that the hardware reason is copied into owned Rust state and that no raw pointer or borrow into `kvm_run` crosses into higher layers;
- `hardware_exit_reason` remains opaque diagnostic metadata and is not interpreted as SGX/VMX policy, retry, recovery, replacement execution, lifecycle action, a new KVM requirement, MMIO, interrupts, SMP, long-mode/Linux boot, migration, resumable execution, or guest-memory/device snapshot behavior;
- generic unsupported raw exit reasons continue to use the existing `Unhandled { reason }` path with register context and are not conflated with KVM's explicit unknown-hardware exit;
- this slice changes documentation only and does not alter the already integrated fail-entry, internal-error, system-event, I/O, budget, CPU/MSR/state, or CLI behavior.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 68 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer KVM-unknown hardware-reason interpretation/recovery, internal-error suberror/data-specific recovery or retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this documentation synchronization boundary.
