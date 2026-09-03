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
- composite vCPU state snapshots that own the existing general-register, special-register, and policy-bound MSR snapshots together, with pure component-preserving comparison, read-only snapshot-bound verification, bounded non-transactional restore, restore-and-verify, and a deterministic public/CLI round-trip fixture;
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_FAIL_ENTRY` and `KVM_EXIT_SYSTEM_EVENT` payload diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, fail-entry, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access;
- public README, architecture, and safety documentation synchronized through the Phase 54 documentation pass.

## Phase 55 — typed KVM fail-entry diagnostics

The current bounded slice preserves KVM's purpose-built vCPU entry-failure diagnostics instead of collapsing raw exit reason `9` into the generic unhandled-exit path. It adds no retry, placement policy, or guest lifecycle behavior.

Correctness contract:

- raw KVM exit reason `9` classifies as `VcpuExit::FailEntry` and round-trips back to reason `9`; existing HLT, port-I/O, legacy `KVM_EXIT_SHUTDOWN`, `KVM_EXIT_SYSTEM_EVENT`, and unknown-reason classification remains unchanged;
- `VcpuFailEntry` owns the raw `hardware_entry_failure_reason` and `cpu` values reported by the x86 KVM fail-entry payload without interpreting either field as a portable policy decision;
- `Vcpu::fail_entry()` exposes that payload only while the current shared `kvm_run` exit reason is `KVM_EXIT_FAIL_ENTRY`; any other reason returns a structured payload-unavailable error;
- the tested x86 fail-entry view uses the KVM union offset `32` and a 48-byte prefix; the existing larger system-event mapping requirement already covers it, while the common mapping-size calculation explicitly includes every typed payload prefix;
- central VM-exit dispatch copies the fail-entry payload and returns structured `EntryFailure` diagnostics without issuing `KVM_GET_REGS` or another vCPU ioctl that could obscure the original entry failure;
- execution bookkeeping records every successfully returned fail-entry exit exactly once before dispatch, and the structured `EntryFailure` receives the full ordered completed-exit trace with reason `9` exactly once at the tail;
- focused public classification plus crate-local ABI-layout, payload-copy, dispatch, and execution-trace regressions lock the boundary without depending on a CI host to reproducibly trigger a hardware entry failure;
- this slice adds no retry, extra `KVM_RUN`, CPU affinity or placement policy, interpretation of architecture-specific failure bits, MMIO, interrupts, SMP, long-mode/Linux boot, migration, guest-memory/device snapshots, resumable execution, or system-event lifecycle policy.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 55 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, or architecture work. Do not infer fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from typed fail-entry diagnostics.
