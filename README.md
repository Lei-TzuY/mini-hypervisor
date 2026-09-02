# mini-hypervisor

`mini-hypervisor` is an x86-64 virtualization and VMM laboratory for Linux hosts. The project is intentionally small, but its boundaries, state transitions, unsafe host interactions, and guest-facing semantics are intended to remain explicit and testable.

## Current capability

The repository currently implements the KVM lifecycle foundation, one bounded guest-RAM region, deterministic guest execution, centralized VM-exit policy, and one minimal port-I/O device path:

- open `/dev/kvm` with structured environment errors;
- require KVM API version 12;
- check `KVM_CAP_USER_MEMORY`, `KVM_CAP_SET_TSS_ADDR`, and `KVM_CAP_SET_IDENTITY_MAP_ADDR`;
- obtain and validate the `kvm_run` mmap size;
- create one VM and configure the x86 KVM identity-map/TSS reserved pages before any vCPU exists;
- create vCPU 0;
- represent guest physical addresses with `GuestPhysAddr`;
- validate, anonymously map, bounds-check, and register one RAM region in KVM slot 0 while rejecting overlap with the reserved x86 KVM pages;
- validate a non-empty flat guest image, its load range, and its entry point before loading it;
- initialize vCPU general registers explicitly for the current real-mode fixtures;
- normalize the six visible real-mode segment bases/selectors to zero and ensure paging/protected mode are disabled;
- classify `KVM_EXIT_HLT`, `KVM_EXIT_IO`, and unsupported raw exit reasons;
- validate x86 `kvm_run` port-I/O metadata and copy OUT payloads only after checked offset/length arithmetic against the mapped region;
- route exits through `vmexit::dispatch_vcpu_exit`;
- service exactly one byte-wide, single-count OUT device at debug port `0xe9` through `PortIoBus`;
- reject unknown ports, IN operations, wide accesses, and multi-count debug-port operations with structured errors;
- respect KVM's pending-I/O completion rule by re-entering `KVM_RUN` before treating execution after an I/O exit as complete;
- produce a typed `VmExitReport` containing vCPU id, terminal exit, RIP, and RFLAGS for handled HLT exits;
- release or conservatively retain mappings according to the documented KVM lifetime rules;
- run pure validation/UAPI/loader/exit-dispatch/port-bus tests without KVM;
- run environment-sensitive KVM integration tests that distinguish unavailable `/dev/kvm` from product regressions.

The deterministic `run-hlt` fixture registers 2 MiB of RAM at guest physical address 0, loads the single byte `HLT` instruction at `0x1000`, starts vCPU 0 there, and expects a handled HLT report with RIP advanced to `0x1001`.

The deterministic `run-debug-port` fixture loads `MOV AL, 'K'; OUT 0xe9, AL; HLT` at `0x1000`. The first completed `KVM_RUN` returns port-I/O metadata and the copied byte `K`; the minimal bus services that output, the VMM re-enters KVM to complete the pending I/O, and execution then terminates at HLT with RIP `0x1005`.

This is still only a single-vCPU flat-binary execution path. It does **not** yet provide port input, MMIO, multiple device families, interrupts, CPUID policy, virtio, snapshots, SMP, ELF loading, or Linux boot support.

## Supported host

- Linux
- x86-64 target architecture
- `/dev/kvm` available and accessible for KVM integration paths
- KVM API version 12
- `KVM_CAP_USER_MEMORY`
- `KVM_CAP_SET_TSS_ADDR`
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

`probe` validates host KVM capabilities. `lifecycle` creates a VM, configures the reserved x86 KVM pages, registers the fixed RAM region, creates vCPU 0, maps `kvm_run`, then shuts down cleanly. `run-hlt` exercises the terminal HLT path. `run-debug-port` additionally exercises one checked KVM port-output exit, the minimal `PortIoBus`, the `0xe9` debug device, I/O completion by re-entry, and final HLT termination.

## Safety boundary

Unsafe operations are limited to Linux KVM `ioctl` calls, conversion of successful KVM-created file descriptors into owned descriptors, and `mmap`/`munmap` for `kvm_run` and guest RAM. Flat guest bytes are copied only through checked guest-memory ranges. The x86 `kvm_run` I/O union is read through tested UAPI layouts, and OUT payloads are copied only after `data_offset + size * count` is checked for conversion, overflow, and mapping bounds. Raw pointers into `kvm_run` never cross into VM-exit policy or device code. No guest physical address is treated as a host pointer.

See [ARCHITECTURE.md](ARCHITECTURE.md), [docs/memory-map.md](docs/memory-map.md), and [docs/safety-assumptions.md](docs/safety-assumptions.md).
