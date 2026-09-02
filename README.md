# mini-hypervisor

`mini-hypervisor` is an x86-64 virtualization and VMM laboratory for Linux hosts. The project is intentionally small, but its boundaries, state transitions, unsafe host interactions, and guest-facing semantics are intended to remain explicit and testable.

## Current capability

The repository currently implements only the first KVM lifecycle slice:

- open `/dev/kvm` with structured environment errors;
- require KVM API version 12;
- check `KVM_CAP_USER_MEMORY` for the planned guest-memory milestone;
- obtain and validate the `kvm_run` mmap size;
- create one VM;
- create vCPU 0;
- map the vCPU `kvm_run` shared structure;
- release mappings and file descriptors through Rust ownership;
- run pure validation tests without KVM;
- run a KVM lifecycle integration test that distinguishes unavailable host capability from product regressions.

It does **not** yet register guest RAM, load or execute guest code, dispatch VM exits, emulate devices, inject interrupts, provide virtio, or support snapshots/SMP.

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

`probe` validates host KVM capabilities. `lifecycle` additionally creates a VM and vCPU and maps `kvm_run`, then shuts down cleanly.

## Safety boundary

The only `unsafe` operations in this milestone are the Linux KVM `ioctl` calls, conversion of successful KVM-created file descriptors into owned descriptors, and `mmap`/`munmap` of the kernel-provided vCPU run structure. They are confined to the KVM/vCPU backend. No guest-controlled pointers or guest memory exist yet.

See [ARCHITECTURE.md](ARCHITECTURE.md) and [docs/safety-assumptions.md](docs/safety-assumptions.md).
