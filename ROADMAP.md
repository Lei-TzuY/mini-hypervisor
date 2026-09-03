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
- centralized VM-exit dispatch, bounded execution budgets, ordered completed-exit reason traces on successful results and budget-exhaustion diagnostics, and the minimal bidirectional debug port-I/O device.

## Phase 44 — budget-exhaustion completed-exit trace diagnostics

The current bounded slice extends the existing `VmExitError::ExitBudgetExhausted` diagnostic with an owned ordered `exit_reasons` trace containing the exits that completed successfully before the budget rejected another run attempt.

Correctness contract:

- the error reuses the same execution-loop trace populated only after successful `Vcpu::run_once()` returns, so failed KVM run attempts cannot appear in the diagnostic;
- every completed exit reason is retained exactly once and in observation order; `completed` equals the trace length and `last_exit_reason` equals the trace tail when one exists;
- a zero exit budget still rejects before any KVM run and therefore carries `completed = 0`, `last_exit_reason = None`, and an empty trace;
- budget exhaustion remains an admission failure before the next `KVM_RUN`; adding the trace does not issue an extra run, retry, dispatch, or state mutation;
- when the last completed exit was serviceable port I/O, its reason is retained even though the budget may prevent the KVM re-entry required to complete that pending I/O operation, preserving the existing pending-I/O caveat;
- the existing `last_exit_reason` field and display text remain available while the new owned trace adds full ordered diagnostics;
- only budget-exhaustion errors gain this trace; other VM-exit and host errors keep their existing contracts, and this slice does not introduce resumable execution or a partial `VmExecutionResult`;
- focused pure regressions lock zero-budget and multi-exit trace invariants, while a KVM-aware regression requires a one-exit debug-port budget to report exactly `[KVM_EXIT_IO]` before rejecting the next run when KVM is available;
- this slice does not add MMIO, interrupts, long-mode boot, SMP, device expansion, migration orchestration, guest-memory/device snapshots, or architectural rollback semantics.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 44 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further execution, architecture-documentation, or state-model work. In particular, do not infer that resumable execution, MMIO, interrupts, long-mode boot, SMP, device expansion, migration orchestration, or guest-memory/device snapshots are automatically next merely because successful and budget-exhausted execution paths now preserve ordered completed-exit reasons.
