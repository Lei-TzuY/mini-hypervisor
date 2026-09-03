# mini-hypervisor

`mini-hypervisor` is an x86-64 virtualization and VMM laboratory for Linux hosts. The project is intentionally small, but its boundaries, state transitions, unsafe host interactions, and guest-facing semantics are intended to remain explicit and testable.

## Current capability

The repository currently implements the KVM lifecycle foundation, one bounded guest-RAM region, an explicit bounded x86 CPUID contract, deterministic guest execution, centralized VM-exit policy, one bounded vCPU execution loop, and one minimal bidirectional port-I/O device path:

- open `/dev/kvm` with structured environment errors;
- require KVM API version 12;
- check `KVM_CAP_USER_MEMORY`, `KVM_CAP_SET_TSS_ADDR`, `KVM_CAP_EXT_CPUID`, and `KVM_CAP_SET_IDENTITY_MAP_ADDR`;
- obtain and validate the `kvm_run` mmap size;
- retrieve KVM's supported x86 CPUID set into one fixed-capacity 256-entry `repr(C)` buffer and reject zero or out-of-capacity returned counts;
- clear LAPIC-dependent x2APIC, TSC-deadline, and KVM PV-unhalt feature bits while this VMM has no in-kernel LAPIC model;
- apply the validated CPUID set with `KVM_SET_CPUID2` before a newly created vCPU is returned to higher layers;
- create one VM and configure the x86 KVM identity-map/TSS reserved pages before any vCPU exists;
- create vCPU 0;
- represent guest physical addresses with `GuestPhysAddr`;
- validate, anonymously map, bounds-check, and register one RAM region in KVM slot 0 while rejecting overlap with the reserved x86 KVM pages;
- validate a non-empty flat guest image, its load range, and its entry point before loading it;
- initialize vCPU general registers explicitly for the current real-mode fixtures;
- normalize the six visible real-mode segment bases/selectors to zero and ensure paging/protected mode are disabled;
- classify `KVM_EXIT_HLT`, `KVM_EXIT_IO`, and unsupported raw exit reasons;
- validate x86 `kvm_run` port-I/O metadata with checked offset/length arithmetic against the mapped region;
- copy OUT payloads into owned Rust memory only after validation;
- write IN responses back into the exact checked `kvm_run` data range only when direction and response length are valid;
- route exits through `vmexit::dispatch_vcpu_exit`;
- repeatedly run and dispatch through `execution::run_vcpu_until_stopped` until a terminal report or explicit VM-exit budget exhaustion;
- preserve completed-exit count, serviced typed port-I/O exits, and the terminal report in `VmExecutionResult`;
- report zero/exhausted budgets with vCPU id, configured budget, completed-exit count, and the last completed raw exit reason when one exists;
- service exactly one byte-wide, single-count debug device at port `0xe9` through `PortIoBus`;
- support configured one-byte IN responses and one-byte OUT capture on that same device;
- reject unknown ports, wide accesses, multi-count debug-port operations, and malformed response/payload sizes with structured errors;
- respect KVM's pending-I/O completion rule by re-entering `KVM_RUN` when execution continues after a serviced I/O exit;
- produce a typed `VmExitReport` containing vCPU id, terminal exit, RIP, and RFLAGS for handled HLT exits;
- release or conservatively retain mappings according to the documented KVM lifetime rules;
- run pure validation/UAPI/CPUID/loader/exit-dispatch/port-bus/execution-budget tests without KVM;
- run environment-sensitive KVM integration tests that distinguish unavailable `/dev/kvm` from product regressions.

The deterministic `run-hlt` fixture registers 2 MiB of RAM at guest physical address 0, loads the single byte `HLT` instruction at `0x1000`, starts vCPU 0 there, and runs with an exit budget of 1. It expects a handled HLT report with RIP advanced to `0x1001`.

The deterministic `run-debug-port` fixture loads `MOV AL, 'K'; OUT 0xe9, AL; HLT` at `0x1000` and runs with an exit budget of 2. The common loop services the port-I/O exit, re-enters KVM to complete the pending OUT, and terminates at HLT with RIP `0x1005`.

The deterministic port-input fixture loads `IN AL, 0xe9; MOV [0x2000], AL; HLT` at `0x1000` and also uses an exit budget of 2. The debug device supplies byte `R`, the vCPU layer writes that response into the exact checked KVM input buffer, the common loop re-enters KVM, and the guest stores the consumed byte into RAM at `0x2000` before halting with RIP `0x1006`.

Exit-budget exhaustion is not a terminal guest report. If the last permitted exit was serviceable I/O, the request has been serviced in userspace but the loop does not claim that KVM has completed the pending operation because no further `KVM_RUN` was permitted.

This is still only a single-vCPU flat-binary execution path. It does **not** yet provide MMIO, multiple device families, interrupts, configurable CPU models, MSR policy, virtio, snapshots, SMP, ELF loading, or Linux boot support.

## Supported host

- Linux
- x86-64 target architecture
- `/dev/kvm` available and accessible for KVM integration paths
- KVM API version 12
- `KVM_CAP_USER_MEMORY`
- `KVM_CAP_SET_TSS_ADDR`
- `KVM_CAP_EXT_CPUID`
- `KVM_CAP_SET_IDENTITY_MAP_ADDR`

GitHub-hosted CI may run without usable `/dev/kvm`; pure tests remain mandatory there, and the environment-sensitive KVM tests report unavailable host capability without treating it as a VMM correctness failure.

## Commands

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo run -- probe
cargo run -- lifecycle
cargo run -- run-hlt
cargo run -- run-debug-port
```

`probe` validates host KVM capabilities and the bounded supported-CPUID query. `lifecycle` creates a VM, configures the reserved x86 KVM pages, registers the fixed RAM region, creates vCPU 0, applies the validated CPUID contract, maps `kvm_run`, then shuts down cleanly. `run-hlt` exercises the bounded terminal HLT path. `run-debug-port` exercises checked KVM port output, the minimal `PortIoBus`, common bounded execution, I/O completion by re-entry, and final HLT termination. The port-input path is currently exercised through the library API and integration regression rather than a separate CLI command.

## Safety boundary

Unsafe operations are limited to Linux KVM `ioctl` calls, conversion of successful KVM-created file descriptors into owned descriptors, and `mmap`/`munmap` for `kvm_run` and guest RAM. The variable-length `kvm_cpuid2` ABI is represented by one contiguous fixed-capacity `repr(C)` header-plus-entry buffer; the returned `nent` is checked before any slice is formed, reserved entry padding is zeroed, and the validated set is copied into a fresh bounded buffer before `KVM_SET_CPUID2`. Flat guest bytes are copied only through checked guest-memory ranges. The x86 `kvm_run` I/O union is accessed only through tested UAPI layouts. Both OUT copying and IN write-back use `data_offset + size * count` only after checked conversion, overflow, and mapping-bounds validation; IN additionally requires an exact response-length match. Raw pointers into `kvm_run` never cross into VM-exit policy, execution-loop, or device code. No guest physical address is treated as a host pointer.

See [ARCHITECTURE.md](ARCHITECTURE.md), [docs/memory-map.md](docs/memory-map.md), and [docs/safety-assumptions.md](docs/safety-assumptions.md).
