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
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_SYSTEM_EVENT` classification and owned payload extraction with structured unsupported diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access.

## Phase 53 — typed KVM system-event payload diagnostics

The current bounded slice adds a typed, owned boundary for `KVM_EXIT_SYSTEM_EVENT` payloads and routes those exits into structured unsupported diagnostics while deliberately leaving reset, reboot, crash, wakeup, suspend, SEV-termination, and TDX-fatal handling policy undefined.

Correctness contract:

- raw KVM exit reason `24` classifies as `VcpuExit::SystemEvent` and round-trips back to reason `24`; existing HLT, port-I/O, legacy `KVM_EXIT_SHUTDOWN`, and unknown-reason classification remains unchanged;
- `VcpuSystemEventType` preserves the current KVM UAPI values for shutdown, reset, crash, wakeup, suspend, SEV termination, and TDX fatal (`1..=7`) while retaining unknown raw values rather than collapsing them;
- `VcpuSystemEvent` owns the decoded event type and only the first `ndata` 64-bit data words reported by KVM; `ndata > 16` is rejected as a structured `InvalidSystemEventDataCount` error before any out-of-bounds payload read;
- `Vcpu::system_event()` exposes payload extraction only when the current shared `kvm_run` exit reason is `KVM_EXIT_SYSTEM_EVENT`; any other reason produces a structured payload-unavailable error;
- the minimum accepted `kvm_run` mapping size now covers both the existing port-I/O prefix and the 168-byte x86 system-event prefix before either typed view may be formed, preserving the safety precondition for raw mmap casts;
- central VM-exit dispatch copies the owned system-event payload, reads the existing vCPU register context, and returns `UnsupportedSystemEvent`; no system-event type is treated as an implemented reset, reboot, crash, shutdown, wakeup, suspend, SEV-termination, or TDX-fatal policy;
- legacy `KVM_EXIT_SHUTDOWN` reason `8` remains the existing typed terminal stop and is intentionally distinct from system-event type `Shutdown` carried inside exit reason `24`;
- execution bookkeeping continues to record every successfully returned KVM exit exactly once before dispatch; `UnsupportedSystemEvent` and malformed-`ndata` diagnostics receive the full ordered completed-exit trace with reason `24` exactly once at the tail;
- focused public classification tests plus crate-local ABI-layout and synthetic payload regressions lock raw values, union offset, payload capacity, ownership, bounds rejection, and trace attachment without requiring a CI guest to reliably trigger a generic KVM system event;
- this slice adds no extra `KVM_RUN`, retry, response writeback, reset/reboot/crash policy, pending-I/O completion, MMIO, interrupts, SMP, long-mode/Linux boot, migration, guest-memory/device snapshots, or resumable execution semantics.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 53 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, architecture-documentation, CPU-model, state-model, memory, or CLI work. Do not infer implemented system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from typed system-event payload diagnostics.
