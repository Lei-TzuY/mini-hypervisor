# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design and may lag the latest merged implementation by one documentation pass; when a historical `Next architectural milestone` paragraph disagrees with this file, use this roadmap for selecting the next slice.

## Current integrated state

The repository currently has typed, owned boundaries for:

- KVM host capability validation, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, and deterministic one-vCPU execution;
- configured guest CPUID derivation, application, read-back verification, guest-observed proof, pure policy comparison, and a deterministic CLI guest-proof fixture;
- bounded host MSR index/feature discovery, feature-value stability classification, immutable host MSR model candidates, and pure candidate comparison;
- explicit guest MSR access policy, policy-validated value sets, policy-bound capture, full MSR snapshots, snapshot comparison, bounded non-transactional restore, and restore-and-verify;
- owned vCPU general-register snapshots, pure 18-field reference-to-observed comparison, snapshot-bound restore, and restore-and-verify;
- owned vCPU special-register snapshots covering segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM UAPI padding, plus pure deterministic semantic-field comparison, snapshot-bound restore, and restore-and-verify;
- composite vCPU state snapshots that own the existing general-register, special-register, and policy-bound MSR snapshots together, with pure component-preserving comparison, bounded non-transactional restore, restore-and-verify, and a deterministic public/CLI round-trip fixture;
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion diagnostics, and unhandled-exit diagnostics, plus the minimal bidirectional debug port-I/O device.

## Phase 49 — guest-observed CPUID CLI proof

The current bounded slice exposes the existing deterministic guest-observed configured-CPUID proof through the CLI without changing the underlying guest program, CPUID policy, feature mask, KVM ABI, or execution semantics.

Correctness contract:

- `run-cpuid` must be recognized as an explicit CLI command and must never fall through to the current unknown-command path;
- the command delegates directly to the existing `run_cpuid_guest(VmConfig::default())` fixture rather than duplicating CPUID policy construction, feature masking, guest machine code, memory decoding, or exit handling;
- when KVM is available, the command reports the guest-observed `CPUID(1).ECX`, `CPUID(0x40000001).EAX`, the existing `masked_lapic_features_clear()` verdict, and the terminal `VmExitReport`;
- the existing fixture remains responsible for executing the reviewed 28-byte real-mode program, reading the checked eight-byte guest-memory result, and proving that x2APIC, TSC-deadline, and KVM PV-unhalt remain clear;
- when `/dev/kvm` is unavailable or inaccessible, the command propagates the existing structured error through the binary's failure path rather than being treated as an unknown successful command;
- the focused CLI regression is environment-independent: it requires command recognition on every runner, checks the structured-error path when KVM is unavailable, and additionally checks the guest-proof output when the command succeeds;
- this slice does not change host CPUID discovery, configured policy derivation/application/read-back, the masked feature set, VM/vCPU lifecycle, guest bytes, execution budgets, terminal-exit policy, MSR/state-snapshot behavior, or any raw KVM representation;
- this slice does not add arbitrary/configurable CPU models, migration compatibility, `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, SMP, long-mode/Linux boot, or resumable execution.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 49 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, architecture-documentation, CPU-model, or state-model work. In particular, do not infer that `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, or another CLI fixture is automatically next merely because the configured-CPUID guest proof is now directly executable.
