# mini-hypervisor

`mini-hypervisor` is an x86-64 virtualization and VMM laboratory for Linux hosts. The project is intentionally small, but its boundaries, state transitions, unsafe host interactions, guest-facing semantics, and verification paths are intended to remain explicit and testable.

## Current capability

The repository currently implements the KVM lifecycle foundation, one bounded guest-RAM region, explicit CPUID/MSR policy and state-model boundaries, deterministic one-vCPU execution, centralized VM-exit policy, bounded execution diagnostics, and one minimal bidirectional port-I/O device path:

- open `/dev/kvm` with structured environment errors;
- require KVM API version 12;
- check required `KVM_CAP_USER_MEMORY`, `KVM_CAP_SET_TSS_ADDR`, `KVM_CAP_EXT_CPUID`, `KVM_CAP_SET_IDENTITY_MAP_ADDR`, and `KVM_CAP_GET_MSR_FEATURES` extensions;
- additionally observe `KVM_CAP_INTERNAL_ERROR_DATA` through `KVM_CHECK_EXTENSION` as optional host metadata without making support a backend requirement;
- obtain and validate the `kvm_run` mmap size;
- retrieve KVM's supported x86 CPUID set into one fixed-capacity 256-entry `repr(C)` buffer and reject zero or out-of-capacity returned counts;
- derive a bounded configured guest CPUID contract that clears LAPIC-dependent x2APIC, TSC-deadline, and KVM PV-unhalt feature bits while this VMM has no in-kernel LAPIC model;
- apply the validated guest CPUID set before a newly created vCPU is returned to higher layers, read it back, compare it against the configured contract, and support guest-observed CPUID proof;
- discover bounded host MSR index/feature sets, classify feature-value stability, build immutable host MSR model candidates, and compare candidates without KVM mutation;
- represent guest MSR access through an explicit validated policy, policy-bound value sets and snapshots, pure snapshot comparison, read-only snapshot-bound verification, bounded non-transactional restore, and restore-and-verify;
- create one VM and configure the x86 KVM identity-map/TSS reserved pages before any vCPU exists;
- create vCPU 0;
- represent guest physical addresses with `GuestPhysAddr`;
- validate, anonymously map, bounds-check, and register one RAM region in KVM slot 0 while rejecting overlap with the reserved x86 KVM pages;
- validate a non-empty flat guest image, its load range, and its entry point before loading it;
- initialize vCPU general registers explicitly for the current real-mode fixtures;
- normalize the six visible real-mode segment bases/selectors to zero and ensure paging/protected mode are disabled;
- capture owned vCPU general-register snapshots, compare all 18 fields deterministically, verify them through fresh read-only capture, restore them, and restore-and-verify through read-back;
- capture owned vCPU special-register snapshots covering segments, descriptor tables, control registers, EFER, APIC base, and the interrupt bitmap without exposing KVM padding, then compare, verify through fresh read-only capture, restore, and restore-and-verify them;
- capture composite vCPU state snapshots containing general registers, special registers, and policy-bound MSRs; compare those components without flattening their typed mismatch semantics; verify against a fresh read-only capture; perform bounded non-transactional restore; and restore-and-verify through a fresh capture;
- classify `KVM_EXIT_HLT`, `KVM_EXIT_IO`, legacy `KVM_EXIT_SHUTDOWN`, `KVM_EXIT_FAIL_ENTRY`, `KVM_EXIT_INTERNAL_ERROR`, and `KVM_EXIT_SYSTEM_EVENT` as typed exits while preserving unsupported raw exit reasons;
- decode `KVM_EXIT_FAIL_ENTRY` through its tested x86 `kvm_run` union layout and copy the raw hardware entry failure reason plus CPU field into owned Rust state;
- decode the always-available base of `KVM_EXIT_INTERNAL_ERROR` through a tested x86 `kvm_run` prefix and copy only `suberror` into owned Rust state without reading capability-dependent `ndata` or `data[16]`;
- decode `KVM_EXIT_SYSTEM_EVENT` through a tested 168-byte x86 `kvm_run` prefix, reject `ndata > 16`, and copy only the declared payload words into owned Rust state;
- validate x86 `kvm_run` port-I/O metadata with checked offset/length arithmetic against the mapped region;
- copy OUT payloads into owned Rust memory only after validation;
- write IN responses back into the exact checked `kvm_run` data range only when direction and response length are valid;
- route exits through `vmexit::dispatch_vcpu_exit`;
- repeatedly run and dispatch through `execution::run_vcpu_until_stopped` until a typed terminal report, structured unsupported/entry-failure/internal-error diagnostic, or explicit VM-exit budget exhaustion;
- preserve completed-exit count, serviced typed port-I/O exits, the terminal report, and the full ordered raw exit-reason trace in successful `VmExecutionResult` values;
- preserve the full ordered completed-exit trace on budget exhaustion while retaining the configured budget, completed count, and last completed reason;
- preserve the full ordered completed-exit trace on unhandled VM exits while retaining vCPU id, raw reason, RIP, and RFLAGS diagnostics;
- preserve the full ordered completed-exit trace on fail-entry diagnostics while retaining the raw hardware entry failure reason and CPU field without issuing a secondary register-read ioctl;
- preserve the full ordered completed-exit trace on internal-error diagnostics while retaining the raw `suberror` without reading optional internal-error data or issuing a secondary register-read ioctl;
- preserve the full ordered completed-exit trace on unsupported or malformed system-event diagnostics while retaining decoded event context or invalid `ndata` metadata;
- service exactly one byte-wide, single-count debug device at port `0xe9` through `PortIoBus`;
- support configured one-byte IN responses and one-byte OUT capture on that same device;
- reject unknown ports, wide accesses, multi-count debug-port operations, and malformed response/payload sizes with structured errors;
- respect KVM's pending-I/O completion rule by re-entering `KVM_RUN` when execution continues after a serviced I/O exit;
- produce a typed `VmExitReport` containing vCPU id, terminal exit, RIP, and RFLAGS for handled HLT and legacy shutdown exits;
- release or conservatively retain mappings according to the documented KVM lifetime rules;
- run pure validation/UAPI/CPUID/MSR/state-snapshot/loader/exit-dispatch/fail-entry/internal-error/system-event/port-bus/execution-budget tests without requiring KVM;
- run environment-sensitive KVM integration tests that distinguish unavailable `/dev/kvm` from product regressions.

The deterministic `run-hlt` fixture registers 2 MiB of RAM at guest physical address 0, loads the single byte `HLT` instruction at `0x1000`, starts vCPU 0 there, and runs with an exit budget of 1. It expects a handled HLT report with RIP advanced to `0x1001`.

The deterministic `run-debug-port` fixture loads `MOV AL, 'K'; OUT 0xe9, AL; HLT` at `0x1000` and runs with an exit budget of 2. The common loop services the port-I/O exit, re-enters KVM to complete the pending OUT, and terminates at HLT with RIP `0x1005`.

The deterministic port-input fixture loads `IN AL, 0xe9; MOV [0x2000], AL; HLT` at `0x1000` and also uses an exit budget of 2. The debug device supplies byte `R`, the vCPU layer writes that response into the exact checked KVM input buffer, the common loop re-enters KVM, and the guest stores the consumed byte into RAM at `0x2000` before halting with RIP `0x1006`.

The deterministic `run-cpuid` fixture executes the existing guest-observed CPUID proof and prints the guest's `CPUID(1).ECX`, `CPUID(0x40000001).EAX`, whether the three currently masked LAPIC-dependent feature bits remain clear, and the terminal HLT report. The CLI reuses the library fixture directly; it does not introduce a second CPUID policy, feature mask, or guest program.

The deterministic `state-roundtrip` fixture creates vCPU 0 without running guest code, uses an intentionally empty guest MSR policy for host portability, captures reference composite CPU state at real-mode RIP `0x1000`, changes the configured state to RIP `0x1200`, proves that the changed snapshot no longer matches, then restores and verifies the original snapshot through the existing bounded composite restore-and-verify path. It reports typed changed/restored comparison results and does not claim whole-VM, guest-memory, device-state, migration, checkpoint, atomic/quiesced snapshot, rollback, or retry semantics.

KVM-aware state regressions also exercise real vCPU capture/compare/verify/restore/restore-and-verify paths when `/dev/kvm` is available. Component-level read-only verification now mirrors the existing composite verification boundary: each operation performs a fresh capture through the reference snapshot's own scope or MSR policy and returns the existing typed comparison without restoring or repairing mismatches. These snapshots cover the owned vCPU CPU-state boundaries listed above; they are **not** whole-VM, guest-memory, device-state, migration, checkpoint, or atomic/quiesced snapshot semantics.

Exit-budget exhaustion is not a terminal guest report. If the last permitted exit was serviceable I/O, the request has been serviced in userspace but the loop does not claim that KVM has completed the pending operation because no further `KVM_RUN` was permitted. Likewise, composite state restore is explicitly non-transactional: if a later component fails, already completed earlier component writes are not rolled back.

`KVM_EXIT_FAIL_ENTRY` is now classified and decoded into owned typed diagnostic state. The VMM preserves KVM's raw `hardware_entry_failure_reason` and `cpu` fields and stops the execution attempt with a structured error; it does not reinterpret those architecture-specific diagnostics into retry, CPU-affinity, placement, or recovery policy and does not issue a secondary register read that could replace the original failure with another error.

`KVM_EXIT_INTERNAL_ERROR` is now classified as a typed exit and decoded only through its always-available base field. The backend additionally records the raw `KVM_CHECK_EXTENSION` observation for `KVM_CAP_INTERNAL_ERROR_DATA`; a missing/zero value is explicitly allowed and can be inspected through `HostCapabilities` without changing backend validity. The VMM still copies only the raw `suberror` into owned diagnostic state and returns a structured error without issuing a secondary register read. It does **not** read capability-dependent `ndata` or `data[16]`, and it does not infer emulation recovery, retry, replacement execution, or architecture-specific policy from the suberror. Optional internal-error payload decoding remains unsupported until a separate capability-aware design is introduced.

`KVM_EXIT_SYSTEM_EVENT` is classified and decoded into owned typed payload state, but handling policy remains deliberately undefined: shutdown/reset/crash/wakeup/suspend/SEV-termination/TDX-fatal events are reported as structured unsupported diagnostics rather than being translated into reboot, termination, or other VM lifecycle actions. This is distinct from legacy `KVM_EXIT_SHUTDOWN`, which remains a typed terminal stop.

This remains a single-vCPU, flat-binary, real-mode execution laboratory. It does **not** yet provide MMIO, multiple device families, interrupts, an in-kernel interrupt controller model, arbitrary/configurable CPU models, virtio, SMP, ELF loading, long-mode guest boot, Linux boot, migration orchestration, whole-VM snapshots, guest-memory/device snapshots, resumable execution, architectural rollback, fail-entry retry/placement policy, optional internal-error payload decoding or recovery policy, or implemented system-event lifecycle policy.

For the authoritative current bounded implementation state and next-slice selection rules, see [ROADMAP.md](ROADMAP.md).

## Supported host

- Linux
- x86-64 target architecture
- `/dev/kvm` available and accessible for KVM integration paths
- KVM API version 12
- `KVM_CAP_USER_MEMORY`
- `KVM_CAP_SET_TSS_ADDR`
- `KVM_CAP_EXT_CPUID`
- `KVM_CAP_SET_IDENTITY_MAP_ADDR`
- `KVM_CAP_GET_MSR_FEATURES`

`KVM_CAP_INTERNAL_ERROR_DATA` is observed when `/dev/kvm` is opened but remains optional; hosts reporting value 0 are still valid for the current base internal-error diagnostic boundary.

GitHub-hosted CI may run without usable `/dev/kvm`; pure tests remain mandatory there, and the environment-sensitive KVM tests report unavailable host capability without treating it as a VMM correctness failure.

## Commands

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo run -- probe
cargo run -- lifecycle
cargo run -- state-roundtrip
cargo run -- run-cpuid
cargo run -- run-hlt
cargo run -- run-debug-port
```

`probe` validates host KVM capabilities and the bounded supported-CPUID query. `lifecycle` creates a VM, configures the reserved x86 KVM pages, registers the fixed RAM region, creates vCPU 0, applies the validated CPUID contract, maps `kvm_run`, then shuts down cleanly. `state-roundtrip` exercises the deterministic composite CPU-state capture/change/restore-and-verify path without executing guest code. `run-cpuid` exposes the existing deterministic guest-observed configured-CPUID proof; when KVM is unavailable or inaccessible it propagates the existing structured environment error instead of falling through as an unknown successful command. `run-hlt` exercises the bounded terminal HLT path. `run-debug-port` exercises checked KVM port output, the minimal `PortIoBus`, common bounded execution, I/O completion by re-entry, and final HLT termination. The port-input path is currently exercised through the library API and integration regression rather than a separate CLI command.

## Safety boundary

Unsafe operations are limited to Linux KVM `ioctl` calls, conversion of successful KVM-created file descriptors into owned descriptors, and `mmap`/`munmap` for `kvm_run` and guest RAM. Variable-length KVM ABIs are represented by bounded `repr(C)` buffers with returned counts validated before slices are formed. Flat guest bytes are copied only through checked guest-memory ranges. The x86 `kvm_run` I/O, fail-entry, internal-error-base, and system-event views are accessed only through tested UAPI layouts and only after the mapping is known large enough for every required prefix. Fail-entry diagnostics are copied immediately into owned scalar state. Internal-error handling copies only the always-available `suberror` and intentionally does not form or consume capability-dependent `ndata`/`data` state even when the optional host capability observation is positive. System-event `ndata` is bounded by the fixed 16-word UAPI capacity before any payload slice is formed. Both OUT copying and IN write-back use `data_offset + size * count` only after checked conversion, overflow, and mapping-bounds validation; IN additionally requires an exact response-length match. Raw pointers into `kvm_run` never cross into VM-exit policy, execution-loop, or device code. No guest physical address is treated as a host pointer.

CPU/MSR snapshot comparison and read-only verification are capture-and-compare operations over owned values and do not invoke restore or setter paths. Restore boundaries delegate to the existing validated KVM setters and deliberately do not claim transactionality, rollback, repair, or atomic point-in-time capture. MSR partial writes retain structured diagnostics for the processed prefix rather than pretending the operation was all-or-nothing.

See [ARCHITECTURE.md](ARCHITECTURE.md), [docs/memory-map.md](docs/memory-map.md), and [docs/safety-assumptions.md](docs/safety-assumptions.md). Architecture and safety documents describe the accumulated design and can lag the authoritative roadmap by one documentation pass.
