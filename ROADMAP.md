# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host capability validation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, and pure policy comparison;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots, pure 18-field reference-to-observed comparison, snapshot-bound restore, and restore-and-verify;
- owned vCPU special-register snapshots covering segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM UAPI padding, plus pure deterministic semantic-field comparison, snapshot-bound restore, and restore-and-verify;
- composite vCPU state snapshots that own the existing general-register, special-register, and policy-bound MSR snapshots together, with pure component-preserving comparison, bounded non-transactional restore, and restore-and-verify;
- centralized VM-exit dispatch with typed HLT and shutdown terminal exits, bounded execution budgets, ordered completed-exit reason traces on successful results and budget-exhaustion diagnostics, and the minimal bidirectional debug port-I/O device.

## Phase 45 — typed KVM shutdown terminal exit

The current bounded slice classifies `KVM_EXIT_SHUTDOWN` as the typed `VcpuExit::Shutdown` terminal condition and routes it through the existing stopped-execution report path instead of treating reason 8 as an unhandled exit.

Correctness contract:

- the private Linux KVM UAPI boundary defines `KVM_EXIT_SHUTDOWN = 8` and a focused UAPI regression locks that value;
- `VcpuExit::from_raw(KVM_EXIT_SHUTDOWN)` returns `VcpuExit::Shutdown`, while `VcpuExit::Shutdown.reason()` returns the same raw reason; HLT, I/O, and unknown-reason classification remain unchanged;
- shutdown is terminal rather than serviceable: dispatch performs no port-I/O parsing or response writeback and returns `VmExitDisposition::Stopped` after capturing the same vCPU id, RIP, and RFLAGS context retained for HLT reports;
- the bounded execution loop records the shutdown reason exactly once through the existing successful-exit bookkeeping path, so a successful shutdown result has a final ordered trace entry equal to the terminal report reason and does not issue an extra `KVM_RUN`;
- a KVM-aware regression uses a real-mode zero-limit IDT followed by `INT3` to induce a triple-fault shutdown when KVM is available, requiring one completed exit, no serviced I/O exits, a typed shutdown terminal report, and an ordered `[KVM_EXIT_SHUTDOWN]` trace;
- unknown KVM exit reasons remain structured `Unhandled` errors with register context; this slice does not broaden terminal handling beyond the legacy shutdown reason;
- this slice does not add `KVM_EXIT_SYSTEM_EVENT` payload handling, reset/crash semantics, MMIO, interrupts, long-mode boot, SMP, device expansion, migration orchestration, guest-memory/device snapshots, resumable execution, or rollback semantics.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 45 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further execution, architecture-documentation, or state-model work. In particular, do not infer that `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, long-mode boot, SMP, device expansion, migration orchestration, or guest-memory/device snapshots are automatically next merely because the legacy KVM shutdown exit is now typed and terminal.
