# mini-hypervisor

`mini-hypervisor` is an x86-64 virtualization and VMM laboratory for Linux hosts. The project is intentionally small, but its boundaries, state transitions, unsafe host interactions, and guest-facing semantics are intended to remain explicit and testable.

## Current capability

The repository currently implements the KVM lifecycle foundation, one bounded guest-RAM region, and the first deterministic guest-execution slice:

- open `/dev/kvm` with structured environment errors;
- require KVM API version 12;
- check `KVM_CAP_USER_MEMORY`;
- obtain and validate the `kvm_run` mmap size;
- create one VM and vCPU 0;
- represent guest physical addresses with `GuestPhysAddr`;
- validate, anonymously map, bounds-check, and register one RAM region in KVM slot 0;
- validate a non-empty flat guest image, its load range, and its entry point before loading it;
- initialize vCPU general registers explicitly for the current real-mode fixture;
- normalize the six visible real-mode segment bases/selectors to zero and ensure paging/protected mode are disabled;
- execute one `KVM_RUN` iteration;
- classify `KVM_EXIT_HLT` explicitly while preserving unsupported exit reason numbers;
- read back RIP/RFLAGS after execution;
- release or conservatively retain mappings according to the documented KVM lifetime rules;
- run pure validation/UAPI/loader/exit-classification tests without KVM;
- run environment-sensitive KVM integration tests that distinguish unavailable `/dev/kvm` from product regressions.

The deterministic `run-hlt` fixture registers 2 MiB of RAM at guest physical address 0, loads the single byte `HLT` instruction at `0x1000`, starts vCPU 0 there, and expects a HLT exit with RIP advanced to `0x1001`.

This is still only a single-vCPU flat-binary execution path. It does **not** yet provide a centralized VM-exit dispatcher, port I/O or MMIO buses, device models, interrupts, CPUID policy, virtio, snapshots, SMP, ELF loading, or Linux boot support.

## Supported host

- Linux
- x86-64 target architecture
- `/dev/kvm` available and accessible for KVM integration paths
- KVM API version 12

GitHub-hosted CI may run without usable `/dev/kvm`; pure tests remain mandatory there, and the environment-sensitive KVM tests report unavailable host capability without treating it as a VMM correctness failure.

## Commands

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo run -- probe
cargo run -- lifecycle
cargo run -- run-hlt
```

`probe` validates host KVM capabilities. `lifecycle` creates a VM, registers the fixed RAM region, creates vCPU 0, maps `kvm_run`, then shuts down cleanly. `run-hlt` additionally loads the deterministic flat guest, initializes vCPU state, executes it once, and prints the classified exit plus RIP/RFLAGS.

## Safety boundary

Unsafe operations are limited to Linux KVM `ioctl` calls, conversion of successful KVM-created file descriptors into owned descriptors, and `mmap`/`munmap` for `kvm_run` and guest RAM. Flat guest bytes are copied only through checked guest-memory ranges. `kvm_run` is read only through a tested UAPI prefix layout after KVM returns from `KVM_RUN`. No guest physical address is treated as a host pointer.

See [ARCHITECTURE.md](ARCHITECTURE.md), [docs/memory-map.md](docs/memory-map.md), and [docs/safety-assumptions.md](docs/safety-assumptions.md).
