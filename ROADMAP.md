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
- centralized VM-exit dispatch, bounded execution budgets, ordered successful-exit reason traces, and the minimal bidirectional debug port-I/O device.

## Phase 43 — ordered successful execution exit-reason trace

The current bounded slice extends `VmExecutionResult` with a read-only ordered `exit_reasons()` trace covering every `KVM_RUN` exit that completed successfully before the terminal result.

Correctness contract:

- a raw exit reason is appended only after `Vcpu::run_once()` returns success; failed `KVM_RUN` attempts therefore cannot create trace entries;
- every successfully returned exit is recorded exactly once and in the same completion order in which the execution loop observes it;
- budget accounting and exit tracing are advanced together through one bookkeeping boundary so the successful-result trace length equals `completed_exits`;
- the terminal exit remains part of the trace, and on every successful `VmExecutionResult` the final trace reason equals `VmExitReport::exit().reason()`;
- existing typed `io_exits()` remains unchanged and continues to retain only serviced typed port-I/O exits rather than becoming a duplicate raw trace;
- VM-exit dispatch, pending-I/O completion by KVM re-entry, exit-budget admission, terminal handling, and error propagation are unchanged; this slice adds no extra `KVM_RUN`, retry, or state mutation;
- exit-budget exhaustion and other error paths still return the existing structured errors rather than a partial `VmExecutionResult`, so this slice does not claim resumable or error-path trace recovery;
- a pure bookkeeping regression locks budget/trace lockstep, while a KVM-aware regression requires the existing debug-port guest to report `[KVM_EXIT_IO, KVM_EXIT_HLT]` in exact order when KVM is available;
- this slice does not add MMIO, interrupts, long-mode boot, SMP, device expansion, migration orchestration, guest-memory/device snapshots, or architectural rollback semantics.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 43 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further execution, architecture-documentation, or state-model work. In particular, do not infer that MMIO, interrupts, long-mode boot, SMP, device expansion, migration orchestration, or guest-memory/device snapshots are automatically next merely because successful execution results now retain an ordered raw exit-reason trace.
