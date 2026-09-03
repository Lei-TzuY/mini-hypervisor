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
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion diagnostics, and unhandled-exit diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access.

## Phase 52 — deterministic unknown-command CLI failure

The current bounded slice fixes the binary command-dispatch contract so an unrecognized command is a process-level usage failure rather than a misleading successful invocation, without introducing CLI-only concepts into the library error taxonomy or changing any hypervisor behavior.

Correctness contract:

- the binary-local `run()` boundary returns an `ExitCode` on successful dispatch while continuing to propagate the existing `mini_hypervisor::error::Error` for known commands that fail in the hypervisor/library layer;
- every recognized command that completes successfully still returns `ExitCode::SUCCESS`, and existing structured hypervisor failures still flow through `main()` as `error: ...` with a non-zero process status;
- an unknown command prints the existing usage line and `unknown command: <value>` to stderr, emits nothing to stdout, and returns the deterministic usage exit code `2`;
- unknown-command handling performs no `KvmBackend::open()`, KVM ioctl, VM/vCPU creation, guest-memory mutation, guest execution, or other hypervisor work;
- the focused binary regression is environment-independent and locks the exact exit code, empty stdout, usage text, unknown-command text, and separation from the structured `error: ...` hypervisor-failure path;
- this slice does not change recognized command names, default `probe` behavior, library error variants, KVM ABI, VM/vCPU lifecycle, execution semantics, CPU/MSR policy, state snapshots, `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, SMP, long-mode/Linux boot, migration, or resumable execution.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 52 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, architecture-documentation, CPU-model, state-model, memory, or CLI work. Do not infer `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from deterministic unknown-command failure semantics.
