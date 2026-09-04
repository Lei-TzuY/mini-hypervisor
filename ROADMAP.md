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
- public README, architecture, and safety documentation synchronized with the integrated Phase 66 typed KVM-unknown diagnostic boundary; the Phase 69 emulation-failure metadata accessors may be reconciled in a later documentation pass.

## Phase 69 — internal-error emulation-failure metadata interpretation

The current bounded slice adds a policy-neutral read-only interpretation of the Linux KVM `KVM_INTERNAL_ERROR_EMULATION` ABI fields that are already present inside capability-gated owned `VcpuInternalError` optional data. It adds no new KVM read, raw pointer, ioctl, dispatch branch, execution policy, recovery behavior, or lifecycle action.

Correctness contract:

- `VcpuInternalError::emulation_failure_flags()` is available only when the suberror class is `Emulation` and at least the first optional data word exists; it returns that raw `u64` flags word exactly and preserves unknown flag bits;
- the Linux KVM `KVM_INTERNAL_ERROR_EMULATION_FLAG_INSTRUCTION_BYTES` bit is treated only as permission to inspect the fixed instruction metadata overlay already contained in owned optional data;
- instruction metadata is considered present only when the suberror is `Emulation`, the instruction-bytes flag is set, and at least three optional `u64` words are owned: one flags word plus the complete fixed 16-byte `insn_size`/`insn_bytes[15]` overlay;
- `emulation_instruction_size()` exposes the raw kernel-reported `u8` size without silently normalizing it;
- `emulation_instruction_bytes()` returns exactly the declared instruction-byte prefix only when the raw size is `<= 15`; an oversized size is never used as a slice length and yields `None` while remaining observable through `emulation_instruction_size()`;
- fixed overlay reconstruction follows the current x86 little-endian UAPI layout and reads only the already-owned optional words; it does not form another `kvm_run` view or extend the unsafe mapping boundary;
- base-only internal-error decoding, non-emulation suberrors, missing optional data, absent instruction-bytes flag, or incomplete optional overlays do not guess metadata and report these read-only views as absent;
- raw `suberror`, `suberror_kind()`, optional `data()`, structured `VmExitError` shapes, dispatch, ordered completed-exit traces, and capability-gating semantics remain unchanged;
- arbitrary trailing debug words and unknown flag bits are not interpreted, and no emulation recovery, retry, replacement execution, instruction emulation, lifecycle action, new KVM requirement, MMIO, interrupts, SMP, long-mode/Linux boot, migration, resumable execution, or guest-memory/device snapshot behavior is introduced;
- focused unit regressions cover capability absence, wrong suberror, unknown flag preservation, incomplete overlays, flag absence, exact instruction-byte extraction, and oversized instruction-size rejection before slicing.

## Next bounded slice

No broader implementation slice is preselected by this commit.

After Phase 69 is integrated and its exact post-merge `main` CI is verified, re-inspect the live repository state, open PRs/issues, recent commits, code/documentation drift, and this authoritative roadmap before selecting further execution, CPU-model, state-model, memory, CLI, lifecycle-policy, capability, or architecture work. Do not infer emulation recovery or instruction emulation, arbitrary internal-error debug-data interpretation, KVM-unknown hardware-reason interpretation/recovery, internal-error suberror-specific retry, fail-entry retry/placement policy, system-event reset/reboot/crash policy, MMIO, interrupts, long-mode boot, SMP, migration, resumable execution, guest-memory/device snapshots, or another CLI command automatically from this read-only diagnostic metadata boundary.
