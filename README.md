# mini-hypervisor

`mini-hypervisor` is an x86-64 virtualization and VMM laboratory for Linux hosts. The project is intentionally small, but its boundaries, state transitions, unsafe host interactions, guest-facing semantics, and verification paths are intended to remain explicit and testable.

## Current capability

The repository currently implements the KVM lifecycle foundation, one bounded guest-RAM region, explicit CPUID/MSR policy and state-model boundaries, deterministic one-vCPU execution, centralized VM-exit policy, bounded execution diagnostics, a minimal bidirectional port-I/O device path, a fixed x86-64 long-mode bootstrap, and one bounded ELF64 executable-loading path:

- open `/dev/kvm` with structured environment errors;
- require KVM API version 12;
- check required `KVM_CAP_USER_MEMORY`, `KVM_CAP_SET_TSS_ADDR`, `KVM_CAP_EXT_CPUID`, `KVM_CAP_SET_IDENTITY_MAP_ADDR`, and `KVM_CAP_GET_MSR_FEATURES` extensions;
- additionally observe `KVM_CAP_INTERNAL_ERROR_DATA` through `KVM_CHECK_EXTENSION` as optional host metadata without making support a backend requirement;
- propagate the optional internal-error-data support bit through VM/vCPU construction without changing the required host contract;
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
- parse and validate bounded ELF64 little-endian x86-64 `ET_EXEC` images, require checked non-overlapping `PT_LOAD` ranges inside the fixed 2 MiB identity map, reject bootstrap-page-table overlap, require an executable file-backed entry point, copy file-backed bytes, and explicitly zero BSS tails;
- initialize vCPU general registers explicitly for the existing real-mode fixtures;
- normalize the six visible real-mode segment bases/selectors to zero and ensure paging/protected mode are disabled for those fixtures;
- validate a fixed long-mode boot layout requiring guest RAM at GPA 0 with at least 2 MiB, an entry inside the mapped extent and outside the reserved page-table pages, and a non-zero mapped stack pointer outside those pages;
- construct the long-mode PML4 at `0x1000`, PDPT at `0x2000`, and PD at `0x3000`, with one 2 MiB identity-mapped large page covering virtual/physical `0..0x20_0000`;
- configure long-mode vCPU state with `CR0.PE|PG`, `CR4.PAE`, `EFER.LME|LMA`, `CR3 = 0x1000`, flat ring-0 64-bit code/data segments, explicit RIP/RSP, and architectural RFLAGS bit 1 while preserving unrelated inherited special-register bits;
- capture owned vCPU general-register snapshots, compare all 18 fields deterministically, verify them through fresh read-only capture, restore them, and restore-and-verify through read-back;
- capture owned vCPU special-register snapshots covering segments, descriptor tables, control registers, EFER, APIC base, and the interrupt bitmap without exposing KVM padding, then compare, verify through fresh read-only capture, restore, and restore-and-verify them;
- capture composite vCPU state snapshots containing general registers, special registers, and policy-bound MSRs; compare those components without flattening their typed mismatch semantics; verify against a fresh read-only capture; perform bounded non-transactional restore; and restore-and-verify through a fresh capture;
- classify `KVM_EXIT_HLT`, `KVM_EXIT_IO`, legacy `KVM_EXIT_SHUTDOWN`, `KVM_EXIT_UNKNOWN`, `KVM_EXIT_EXCEPTION`, `KVM_EXIT_FAIL_ENTRY`, `KVM_EXIT_INTERNAL_ERROR`, and `KVM_EXIT_SYSTEM_EVENT` as typed exits while preserving other unsupported raw exit reasons;
- decode `KVM_EXIT_UNKNOWN` through its tested fixed x86 `kvm_run` union layout and copy the raw `hardware_exit_reason` into owned `VcpuKvmUnknownExit` state;
- decode `KVM_EXIT_EXCEPTION` through its tested fixed x86 `kvm_run` union layout and copy the raw exception vector plus error code into owned `VcpuException` state;
- decode `KVM_EXIT_FAIL_ENTRY` through its tested x86 `kvm_run` union layout and copy the raw hardware entry failure reason plus CPU field into owned Rust state;
- always decode the `KVM_EXIT_INTERNAL_ERROR` base raw `suberror`, expose the four currently defined Linux KVM values through lossless `VcpuInternalErrorSuberror` classification while preserving unknown raw values, and, when `KVM_CAP_INTERNAL_ERROR_DATA` is available, use the tested full x86 layout, reject `ndata > 16`, and copy only the declared optional words into owned Rust state;
- for `VcpuInternalErrorSuberror::Emulation`, expose the owned optional-data flags word and the stable instruction-byte overlay only when the Linux KVM flag/layout preconditions are satisfied, while rejecting an oversized instruction size before slicing;
- decode `KVM_EXIT_SYSTEM_EVENT` through a tested 168-byte x86 `kvm_run` prefix, reject `ndata > 16`, and copy only the declared payload words into owned Rust state;
- validate x86 `kvm_run` port-I/O metadata with checked offset/length arithmetic against the mapped region;
- copy OUT payloads into owned Rust memory only after validation;
- write IN responses back into the exact checked `kvm_run` data range only when direction and response length are valid;
- route exits through `vmexit::dispatch_vcpu_exit`;
- repeatedly run and dispatch through `execution::run_vcpu_until_stopped` until a typed terminal report, structured unsupported/KVM-unknown/exception/entry-failure/internal-error diagnostic, or explicit VM-exit budget exhaustion;
- preserve completed-exit count, serviced typed port-I/O exits, the terminal report, and the full ordered raw exit-reason trace in successful `VmExecutionResult` values;
- preserve the full ordered completed-exit trace on budget exhaustion while retaining the configured budget, completed count, and last completed reason;
- preserve the full ordered completed-exit trace on generic unhandled VM exits while retaining vCPU id, raw reason, RIP, and RFLAGS diagnostics;
- preserve the full ordered completed-exit trace on `KVM_EXIT_UNKNOWN` diagnostics while retaining the raw `hardware_exit_reason` without issuing a secondary register-read ioctl;
- preserve the full ordered completed-exit trace on exception diagnostics while retaining the raw exception vector and error code without issuing a secondary register-read ioctl;
- preserve the full ordered completed-exit trace on fail-entry diagnostics while retaining the raw hardware entry failure reason and CPU field without issuing a secondary register-read ioctl;
- preserve the full ordered completed-exit trace on internal-error diagnostics while retaining the raw `suberror`, capability-gated optional data when available, or malformed optional-data count metadata without issuing a secondary register-read ioctl;
- preserve the full ordered completed-exit trace on unsupported or malformed system-event diagnostics while retaining decoded event context or invalid `ndata` metadata;
- service exactly one byte-wide, single-count debug device at port `0xe9` through `PortIoBus`;
- support configured one-byte IN responses and one-byte OUT capture on that same device;
- reject unknown ports, wide accesses, multi-count debug-port operations, and malformed response/payload sizes with structured errors;
- respect KVM's pending-I/O completion rule by re-entering `KVM_RUN` when execution continues after a serviced I/O exit;
- produce a typed `VmExitReport` containing vCPU id, terminal exit, RIP, and RFLAGS for handled HLT and legacy shutdown exits;
- release or conservatively retain mappings according to the documented KVM lifetime rules;
- run pure validation/UAPI/CPUID/MSR/state-snapshot/flat-loader/ELF64-loader/long-mode/exit-dispatch/KVM-unknown/exception/fail-entry/internal-error/system-event/port-bus/execution-budget tests without requiring KVM;
- run environment-sensitive KVM integration tests, plus strict CI long-mode and ELF64 proofs that require usable `/dev/kvm` and fail rather than skipping when either milestone fixture cannot execute.

The deterministic `run-hlt` fixture registers 2 MiB of RAM at guest physical address 0, loads the single byte `HLT` instruction at `0x1000`, starts vCPU 0 there, and runs with an exit budget of 1. It expects a handled HLT report with RIP advanced to `0x1001`.

The deterministic `run-debug-port` fixture loads `MOV AL, 'K'; OUT 0xe9, AL; HLT` at `0x1000` and runs with an exit budget of 2. The common loop services the port-I/O exit, re-enters KVM to complete the pending OUT, and terminates at HLT with RIP `0x1005`.

The deterministic port-input fixture loads `IN AL, 0xe9; MOV [0x2000], AL; HLT` at `0x1000` and also uses an exit budget of 2. The debug device supplies byte `R`, the vCPU layer writes that response into the exact checked KVM input buffer, the common loop re-enters KVM, and the guest stores the consumed byte into RAM at `0x2000` before halting with RIP `0x1006`.

The deterministic `run-cpuid` fixture executes the existing guest-observed CPUID proof and prints the guest's `CPUID(1).ECX`, `CPUID(0x40000001).EAX`, whether the three currently masked LAPIC-dependent feature bits remain clear, and the terminal HLT report. The CLI reuses the library fixture directly; it does not introduce a second CPUID policy, feature mask, or guest program.

The deterministic `run-long-mode` fixture uses the same 2 MiB RAM region but reserves `0x1000..0x4000` for PML4/PDPT/PD bootstrap pages, loads a reviewed 36-byte x86-64 flat binary at GPA/VA `0x10000`, starts with RSP `0x1ff000`, and runs with an exit budget of 5. The guest uses 64-bit `REX.W` instructions, emits exactly `LM64` through four byte-wide OUT operations on port `0xe9`, then executes HLT. A successful terminal report has RIP `0x10024`. This proves the fixture truly executes in x86-64 long mode.

The deterministic `run-elf64` fixture constructs one bounded ELF64 `ET_EXEC` image with a single executable `PT_LOAD` segment at GPA/VA `0x10000`, places the 36-byte long-mode proof program at ELF entry `0x10100`, gives the segment a larger memory size so the production loader must zero a BSS tail, and then executes that validated entry through the same long-mode bootstrap. Success requires exactly `LM64` through four debug-port exits followed by HLT at RIP `0x10124`. This proves ELF parsing, segment materialization, BSS semantics, long-mode entry, port I/O, and terminal execution in one path; it does not imply relocation, PIE, dynamic linking, or Linux boot support.

The deterministic `state-roundtrip` fixture creates vCPU 0 without running guest code, uses an intentionally empty guest MSR policy for host portability, captures reference composite CPU state at real-mode RIP `0x1000`, changes the configured state to RIP `0x1200`, proves that the changed snapshot no longer matches, then restores and verifies the original snapshot through the existing bounded composite restore-and-verify path. It reports typed changed/restored comparison results and does not claim whole-VM, guest-memory, device-state, migration, checkpoint, atomic/quiesced snapshot, rollback, or retry semantics.

KVM-aware state regressions also exercise real vCPU capture/compare/verify/restore/restore-and-verify paths when `/dev/kvm` is available. Component-level read-only verification mirrors the existing composite verification boundary: each operation performs a fresh capture through the reference snapshot's own scope or MSR policy and returns the existing typed comparison without restoring or repairing mismatches. These snapshots cover the owned vCPU CPU-state boundaries listed above; they are **not** whole-VM, guest-memory, device-state, migration, checkpoint, or atomic/quiesced snapshot semantics.

Exit-budget exhaustion is not a terminal guest report. If the last permitted exit was serviceable I/O, the request has been serviced in userspace but the loop does not claim that KVM has completed the pending operation because no further `KVM_RUN` was permitted. Likewise, composite state restore is explicitly non-transactional: if a later component fails, already completed earlier component writes are not rolled back.

`KVM_EXIT_UNKNOWN` is distinct from this project's generic unsupported raw-reason path. Raw reason `0` maps to `VcpuExit::KvmUnknown`; `Vcpu::kvm_unknown_exit()` validates that reason before reading the fixed x86 union member and copies the opaque `hardware_exit_reason` into owned state. Central dispatch returns structured `VmExitError::KvmUnknownExit` without issuing `KVM_GET_REGS` or another secondary vCPU ioctl, and the common execution loop retains reason `0` exactly once at the tail of the completed-exit trace. The hardware reason remains opaque diagnostic metadata: it is not translated into SGX/VMX interpretation, retry, recovery, replacement execution, or lifecycle policy. Other unsupported raw reasons continue through the generic `Unhandled { reason }` path with RIP/RFLAGS context.

`KVM_EXIT_EXCEPTION` is a policy-neutral typed diagnostic. Raw reason `1` maps to `VcpuExit::Exception`; `Vcpu::exception_exit()` validates that reason before reading the fixed x86 union member and copies the raw `exception: u32` and `error_code: u32` into owned `VcpuException` state. The tested payload is 8 bytes at union offset 32, so its complete prefix is 40 bytes and does not enlarge the existing common `kvm_run` mapping floor. Central dispatch returns structured `VmExitError::Exception` directly from that purpose-built payload without issuing `KVM_GET_REGS` or another secondary vCPU ioctl, and the common execution loop retains reason `1` exactly once at the tail of the completed-exit trace. The vector and error code remain opaque diagnostics: this boundary does not inject or reinject an exception, retry execution, recover the guest, or infer architecture-specific lifecycle policy.

`KVM_EXIT_FAIL_ENTRY` is classified and decoded into owned typed diagnostic state. The VMM preserves KVM's raw `hardware_entry_failure_reason` and `cpu` fields and stops the execution attempt with a structured error; it does not reinterpret those architecture-specific diagnostics into retry, CPU-affinity, placement, or recovery policy and does not issue a secondary register read that could replace the original failure with another error.

`KVM_EXIT_INTERNAL_ERROR` remains a policy-neutral typed diagnostic. The backend records the raw `KVM_CHECK_EXTENSION` observation for `KVM_CAP_INTERNAL_ERROR_DATA` and propagates its positive/zero support state into each created vCPU without making the capability required. Every internal error still owns the always-available raw `suberror`, and `VcpuInternalError::suberror()` continues to expose that exact value. `VcpuInternalError::suberror_kind()` adds a read-only `VcpuInternalErrorSuberror` view: Linux KVM values 1 through 4 map to `Emulation`, `SimultaneousExceptions`, `DeliveryEvent`, and `UnexpectedExitReason`, while every other value remains losslessly available as `Unknown(raw)` and round-trips through `raw()`. That classification is identical on the base-only and optional-data paths and does not interpret any data word. When optional data support is unavailable, `VcpuInternalError::data()` returns `None` and the decoder deliberately stops at the base field. When support is available, the decoder forms the fixed full x86 payload view, validates `ndata <= 16` before any slice is formed, and copies exactly the declared words; `ndata == 0` is represented distinctly as `Some(&[])`. A malformed count becomes a structured diagnostic retaining raw `suberror`, `ndata`, fixed capacity, and the completed-exit trace.

For the `Emulation` suberror only, the owned optional-data words also expose policy-neutral read-only metadata from the stable Linux KVM ABI. `emulation_failure_flags()` returns the first owned data word exactly when it exists and preserves unknown flag bits. The `KVM_INTERNAL_ERROR_EMULATION_FLAG_INSTRUCTION_BYTES` bit permits inspection of the fixed instruction overlay only when at least three owned words are present. `emulation_instruction_size()` returns the raw kernel-reported `u8` size, while `emulation_instruction_bytes()` returns the declared prefix only when that size is at most 15; an oversized size remains observable but yields no byte slice. Missing optional data, a non-emulation suberror, a missing instruction-bytes flag, or an incomplete overlay yields no guessed metadata. These accessors read only already-owned diagnostic words, add no `kvm_run` view or ioctl, and do not imply instruction emulation, recovery, retry, replacement execution, lifecycle action, or interpretation of arbitrary trailing debug data.

`KVM_EXIT_SYSTEM_EVENT` is classified and decoded into owned typed payload state, but handling policy remains deliberately undefined: shutdown/reset/crash/wakeup/suspend/SEV-termination/TDX-fatal events are reported as structured unsupported diagnostics rather than being translated into reboot, termination, or other VM lifecycle actions. This is distinct from legacy `KVM_EXIT_SHUTDOWN`, which remains a typed terminal stop.

This remains a single-vCPU x86 execution laboratory. It provides one fixed deterministic x86-64 long-mode bootstrap and one bounded identity-mapped ELF64 `ET_EXEC` loader/execution path, but it does **not** provide a general virtual-memory manager, ELF relocations, `ET_DYN`/PIE, dynamic linking, a general executable address-layout policy, MMIO, multiple device families, interrupts, an in-kernel interrupt controller model, arbitrary/configurable CPU models, virtio, SMP, Linux boot, migration orchestration, whole-VM snapshots, guest-memory/device snapshots, resumable execution, architectural rollback, exception injection/recovery policy, KVM-unknown hardware-reason recovery policy, fail-entry retry/placement policy, internal-error recovery/retry policy, instruction emulation, or implemented system-event lifecycle policy.

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

`KVM_CAP_INTERNAL_ERROR_DATA` is observed when `/dev/kvm` is opened but remains optional; hosts reporting value 0 are still valid and use the suberror-only internal-error boundary, while positive observations enable bounded optional diagnostic decoding on created vCPUs.

Most environment-sensitive integration tests continue to distinguish unavailable `/dev/kvm` from product regressions. The strict CI gates directly run both `run-long-mode` and `run-elf64`; those gates require a usable KVM device and must observe the exact `LM64` proof plus each fixture's exact terminal HLT RIP and architectural RFLAGS bit 1. KVM unavailability is not milestone success.

## Commands

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo +1.74.0 check --all-targets --all-features
cargo run -- probe
cargo run -- lifecycle
cargo run -- state-roundtrip
cargo run -- run-cpuid
cargo run -- run-hlt
cargo run -- run-debug-port
cargo run -- run-long-mode
cargo run -- run-elf64
```

`probe` validates host KVM capabilities and the bounded supported-CPUID query. `lifecycle` creates a VM, configures the reserved x86 KVM pages, registers the fixed RAM region, creates vCPU 0, applies the validated CPUID contract, maps `kvm_run`, then shuts down cleanly. `state-roundtrip` exercises the deterministic composite CPU-state capture/change/restore-and-verify path without executing guest code. `run-cpuid` exposes the existing deterministic guest-observed configured-CPUID proof; when KVM is unavailable or inaccessible it propagates the existing structured environment error instead of falling through as an unknown successful command. `run-hlt` exercises the bounded terminal HLT path. `run-debug-port` exercises checked KVM port output, the minimal `PortIoBus`, common bounded execution, I/O completion by re-entry, and final HLT termination. `run-long-mode` installs the fixed bootstrap tables/state, executes the deterministic 64-bit flat proof fixture, captures `LM64` through the same port-I/O path, and requires final HLT. `run-elf64` parses and materializes the deterministic ELF64 executable through the production loader, executes its validated entry through the same long-mode path, captures `LM64`, and requires the exact terminal HLT contract. The port-input path is exercised through the library API and integration regression rather than a separate CLI command.

## Safety boundary

Unsafe operations are limited to Linux KVM `ioctl` calls, conversion of successful KVM-created file descriptors into owned descriptors, and `mmap`/`munmap` for `kvm_run` and guest RAM. Variable-length KVM ABIs are represented by bounded `repr(C)` buffers with returned counts validated before slices are formed. Flat guest bytes and long-mode bootstrap page tables are copied only through checked guest-memory ranges. ELF64 bytes and all file-provided offsets, sizes, addresses, alignments, and program-header counts are treated as untrusted: the complete program-header table and every load segment are checked before slices or guest writes are formed; the current loader rejects non-identity mappings, out-of-map ranges, bootstrap-page-table overlap, load-segment overlap, invalid file/memory size relationships, and non-file-backed executable entries. BSS bytes are explicitly zeroed through checked `GuestMemory::write` operations. The long-mode layout does not cast guest physical addresses into host pointers: its fixed PML4/PDPT/PD pages are zeroed and populated through `GuestMemory::write`, and its entry/stack layout is validated before KVM state is configured. The x86 `kvm_run` I/O, KVM-unknown, exception, fail-entry, internal-error, and system-event views are accessed only through tested UAPI layouts and only after the mapping is known large enough for every required prefix. KVM-unknown, exception, and fail-entry diagnostics are copied immediately into owned scalar state. Exception decoding forms only the fixed 40-byte prefix and does not enlarge the existing 168-byte common mapping floor. Internal-error handling always copies the base raw `suberror`; typed suberror classification is a pure view over that copied scalar and never expands the unsafe mapping boundary. The full fixed optional-data view is formed only when the propagated host capability is positive, `ndata > 16` is rejected before slicing, and only declared words are copied into owned state. On hosts without the capability, the base-only decoder never forms or reads the optional fields. Emulation-failure metadata accessors operate only on those already-owned optional words: they require the typed `Emulation` suberror and the documented flag/overlay preconditions, preserve unknown flag bits, and reject an instruction size above the fixed 15-byte capacity before forming a byte slice. They do not form a second `kvm_run` view or extend the unsafe mapping boundary. System-event `ndata` is likewise bounded by the fixed 16-word UAPI capacity before any payload slice is formed. Both OUT copying and IN write-back use `data_offset + size * count` only after checked conversion, overflow, and mapping-bounds validation; IN additionally requires an exact response-length match. Raw pointers into `kvm_run` never cross into VM-exit policy, execution-loop, or device code. No guest physical address is treated as a host pointer.

CPU/MSR snapshot comparison and read-only verification are capture-and-compare operations over owned values and do not invoke restore or setter paths. Restore boundaries delegate to the existing validated KVM setters and deliberately do not claim transactionality, rollback, repair, or atomic point-in-time capture. MSR partial writes retain structured diagnostics for the processed prefix rather than pretending the operation was all-or-nothing.

See [ARCHITECTURE.md](ARCHITECTURE.md), [docs/memory-map.md](docs/memory-map.md), and [docs/safety-assumptions.md](docs/safety-assumptions.md). `ROADMAP.md` is the authoritative source for the bounded milestone boundary and phase-promotion rule.
