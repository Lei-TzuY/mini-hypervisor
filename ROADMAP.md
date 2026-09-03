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
- composite vCPU state snapshots that own the existing general-register, special-register, and policy-bound MSR snapshots together, with pure component-preserving comparison, bounded non-transactional restore, restore-and-verify, and a deterministic public/CLI round-trip fixture;
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion diagnostics, and unhandled-exit diagnostics, plus the minimal bidirectional debug port-I/O device.

## Phase 48 — deterministic composite state round-trip fixture

The current bounded slice exposes the already-integrated composite vCPU state capture/compare/restore/restore-and-verify boundary through one deterministic public fixture and CLI command without changing the underlying state semantics or KVM ABI.

Correctness contract:

- `run_state_snapshot_roundtrip` creates one configured vCPU and deliberately uses an empty `GuestMsrAccessPolicy`, avoiding assumptions that any host-specific MSR exposed by the general capability list is safe to write back in a portable fixture;
- the fixture initializes real-mode state at RIP `0x1000`, captures one reference composite snapshot, reinitializes state at RIP `0x1200`, captures the changed state, and returns the resulting typed comparison as proof that the fixture actually changed state before restore;
- the changed comparison must be non-exact in the focused KVM-aware regression; the fixture does not mutate guest memory, execute guest code, or issue `KVM_RUN`;
- the fixture then calls the existing `restore_and_verify_state_snapshot` boundary exactly once and returns its typed composite comparison without retry, repair, rollback, or new recapture policy beyond that existing API;
- a successful round-trip regression requires the restored composite comparison and its general-register, special-register, and MSR component comparisons all to be exact;
- the public result retains both the changed and restored typed comparisons so callers can distinguish proof-of-mutation from proof-of-restoration rather than receiving only a boolean;
- the `state-roundtrip` CLI command reports the exactness of both comparisons and the three restored components; a mismatch remains a comparison result rather than being redefined as a new error condition;
- the empty MSR policy means the fixture validates the composite orchestration while intentionally not claiming host-portable write semantics for arbitrary MSRs;
- this slice does not add whole-VM, guest-memory, device-state, checkpoint, migration, atomic/quiesced snapshot, transaction, rollback, retry, MMIO, interrupt, SMP, long-mode/Linux boot, resumable execution, or `KVM_EXIT_SYSTEM_EVENT` semantics.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 48 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, architecture-documentation, or state-model work. Do not infer that `KVM_EXIT_SYSTEM_EVENT`, MMIO, interrupts, long-mode boot, SMP, migration, or resumable execution are automatically next merely because composite state round-trip is now directly executable from the CLI.
