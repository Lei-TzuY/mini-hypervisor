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
- centralized VM-exit dispatch with typed HLT and shutdown terminal exits, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion diagnostics, and unhandled-exit diagnostics, plus the minimal bidirectional debug port-I/O device.

## Phase 46 — unhandled-exit completed trace diagnostics

The current bounded slice extends `VmExitError::Unhandled` with an owned ordered `exit_reasons` trace so an unsupported VM exit no longer discards the successful KVM exits observed before dispatch rejected the current reason.

Correctness contract:

- `VmExitError::Unhandled` owns an `exit_reasons: Vec<u32>` while preserving the existing vCPU id, raw reason, RIP, RFLAGS, and display text;
- direct dispatch of one unknown `VcpuExit` produces a local trace containing exactly that raw reason, because that exit has already completed before dispatch begins;
- `run_vcpu_until_stopped` continues recording each successful `Vcpu::run_once()` result before dispatch and, only when dispatch returns `Unhandled`, replaces the local one-element trace with the execution loop's complete ordered trace;
- the execution trace contains every successful KVM exit exactly once, including the current unhandled reason as its final entry; no failed KVM run can appear in it;
- trace attachment does not issue another `KVM_RUN`, retry dispatch, alter exit-budget accounting, service or replay port I/O, or change HLT/shutdown terminal behavior;
- non-`Unhandled` dispatch errors are propagated unchanged and do not gain this trace in this slice;
- focused public-surface and pure regressions lock owned trace storage, direct-dispatch local trace behavior, full-trace replacement, and unchanged propagation of other dispatch errors;
- this slice does not add `KVM_EXIT_SYSTEM_EVENT` payload handling, MMIO, interrupts, long-mode boot, SMP, device expansion, migration orchestration, resumable execution, guest-memory/device snapshots, or rollback semantics.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 46 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, and this authoritative roadmap before choosing further execution, architecture-documentation, or state-model work. Do not infer that `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, long-mode boot, SMP, migration, or resumable execution are automatically next merely because successful, budget-exhausted, and unhandled execution paths now retain ordered completed-exit diagnostics.
