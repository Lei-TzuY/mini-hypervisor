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
- centralized VM-exit dispatch with typed HLT and legacy shutdown terminal exits, typed `KVM_EXIT_UNKNOWN` hardware diagnostics, typed `KVM_EXIT_FAIL_ENTRY`, capability-gated `KVM_EXIT_INTERNAL_ERROR` optional diagnostics plus lossless typed classification of the four currently defined KVM internal-error suberrors and a read-only interpretation of the stable `KVM_INTERNAL_ERROR_EMULATION` flags/instruction-byte overlay, and `KVM_EXIT_SYSTEM_EVENT` diagnostics, bounded execution budgets, ordered completed-exit reason traces on successful results, budget-exhaustion, unhandled-exit, KVM-unknown, fail-entry, internal-error, malformed internal-error-data, and system-event diagnostics, plus the minimal bidirectional debug port-I/O device;
- deterministic CLI command dispatch that preserves structured hypervisor failures for known commands and rejects unknown commands with a usage failure before any KVM access;
- public README, architecture, and safety documentation synchronized with the integrated Phase 69 read-only emulation-failure metadata accessor boundary.

## Phase 71 — emulation-failure architecture and safety documentation

The current bounded slice reconciles `ARCHITECTURE.md` and `docs/safety-assumptions.md` with the already integrated Phase 69 read-only `KVM_INTERNAL_ERROR_EMULATION` metadata accessors and the Phase 70 public README contract. It changes no Rust source, test source, KVM UAPI handling, optional-capability semantics, dispatch, execution policy, error shape, state mutation, or guest lifecycle behavior.

Correctness contract:

- architecture documentation records that `VcpuInternalError::emulation_failure_flags()` is available only for the typed `Emulation` suberror when at least the first owned optional-data word exists, and preserves the complete raw `u64` flags word including unknown bits;
- `KVM_INTERNAL_ERROR_EMULATION_FLAG_INSTRUCTION_BYTES` is documented as authorizing interpretation only of the stable fixed instruction metadata overlay already present in owned optional data;
- the instruction overlay remains unavailable unless the suberror is `Emulation`, the instruction-bytes flag is set, and at least three owned optional words exist;
- `emulation_instruction_size()` exposes the raw kernel-reported `u8` without normalization, while `emulation_instruction_bytes()` returns only the declared prefix for sizes `<= 15`; an oversized size remains observable but never becomes a Rust slice length;
- safety documentation states that these helpers inspect only already-owned `VcpuInternalError` data, form no additional `kvm_run` view, issue no ioctl, borrow no shared mapping, and extend no unsafe mapping lifetime;
- raw `suberror`, lossless `suberror_kind()`, capability-gated `data()`, structured `VmExitError` shapes, dispatch, ordered completed-exit traces, and all runtime behavior remain unchanged;
- arbitrary trailing debug words and unknown emulation flags remain uninterpreted;
- no instruction emulation, emulation recovery, retry, replacement execution, lifecycle action, new KVM requirement, MMIO, interrupts, SMP, long-mode/Linux boot, migration, resumable execution, or guest-memory/device snapshot behavior is introduced;
- validation for this documentation-only slice is exact factual diff review plus the repository's unchanged Format/Clippy/Test CI rather than brittle documentation string tests.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 71 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer emulation recovery or instruction emulation, arbitrary internal-error debug-data interpretation, KVM-unknown hardware-reason interpretation/recovery, internal-error suberror-specific retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this documentation boundary.
