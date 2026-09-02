# mini-hypervisor

`mini-hypervisor` is an x86-64 virtualization and VMM laboratory for Linux hosts. The project is intentionally small, but its boundaries, state transitions, unsafe host interactions, and guest-facing semantics are intended to remain explicit and testable.

## Current capability

The repository currently implements the KVM lifecycle foundation plus one bounded guest-RAM slice:

- open `/dev/kvm` with structured environment errors;
- require KVM API version 12;
- check `KVM_CAP_USER_MEMORY`;
- obtain and validate the `kvm_run` mmap size;
- create one VM and vCPU 0;
- map the vCPU `kvm_run` shared structure;
- represent guest physical addresses with `GuestPhysAddr`;
- validate one page-aligned, non-zero guest RAM region without address-space wraparound;
- anonymously map host RAM and provide bounds-checked guest reads/writes;
- register RAM in KVM memory slot 0 with `KVM_SET_USER_MEMORY_REGION`;
- retain the RAM mapping inside `Vm` for at least the lifetime of the KVM registration;
- run pure boundary/overflow/overlap tests without KVM;
- run a KVM lifecycle integration test that distinguishes unavailable host capability from product regressions.

The deterministic `lifecycle` command currently registers 2 MiB of RAM at guest physical address 0 before creating vCPU 0.

It does **not** yet load or execute guest code, dispatch VM exits, emulate devices, inject interrupts, provide virtio, or support snapshots/SMP.

## Supported host

- Linux
- x86-64 target architecture
- `/dev/kvm` available and accessible
- KVM API version 12

GitHub-hosted CI is expected to run without usable `/dev/kvm`; pure tests remain mandatory there, and the environment-sensitive lifecycle test reports the unavailable capability without treating it as a VMM correctness failure.

## Commands

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo run -- probe
cargo run -- lifecycle
```

`probe` validates host KVM capabilities. `lifecycle` creates a VM, allocates and registers the fixed test RAM region, creates vCPU 0, maps `kvm_run`, then shuts down cleanly.

## Safety boundary

Unsafe operations are limited to Linux KVM `ioctl` calls, conversion of successful KVM-created file descriptors into owned descriptors, and `mmap`/`munmap` for `kvm_run` and guest RAM. Guest-memory copies are performed only after checked guest-address/length validation. No unchecked guest pointer is dereferenced.

See [ARCHITECTURE.md](ARCHITECTURE.md), [docs/memory-map.md](docs/memory-map.md), and [docs/safety-assumptions.md](docs/safety-assumptions.md).
