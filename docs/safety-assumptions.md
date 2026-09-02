# Safety assumptions

## Trust model

The Linux KVM kernel interface and explicitly supplied host process configuration are trusted. Guest-controlled addresses, lengths, future CPU state, and future device requests are untrusted.

The current one-byte HLT fixture is repository-owned test input rather than arbitrary external guest content. `FlatGuestImage` nevertheless applies range and entry validation so later callers do not acquire an unchecked loading path by accident.

## Unsafe boundary in the current milestone

`src/kvm/sys.rs` contains raw `ioctl` calls. The call sites use constants and structures matching the stable x86-64 Linux KVM UAPI and convert `-1` into `std::io::Error` immediately.

Successful `KVM_CREATE_VM` and `KVM_CREATE_VCPU` results are file descriptors owned by the caller; they are immediately wrapped in `OwnedFd`.

Before any vCPU exists, the VM backend configures KVM's x86 identity-map page and TSS pages in the reserved range `0xfeff_c000..0xff00_0000`. The required KVM capabilities are checked before VM creation, and guest RAM registration rejects overlap with this range.

The vCPU run structure is mapped using the exact positive size returned by `KVM_GET_VCPU_MMAP_SIZE`. The mapping must be at least large enough for the tested `KvmRunHeader` prefix before it is accepted. After a successful `KVM_RUN`, only the prefix field containing `exit_reason` is read by the vCPU layer in this milestone.

Guest RAM is a private anonymous mapping owned by `GuestMemory`. Region construction validates non-zero size, 4 KiB alignment, and guest-physical end arithmetic before `mmap` or KVM registration. After `KVM_SET_USER_MEMORY_REGION` succeeds, the `Vm` owns the mapping.

Before a `Vm` releases registered RAM, it explicitly removes KVM slot 0 with a zero-sized memory-region update. A separate vCPU fd may keep the kernel VM alive after the userspace VM handle begins destruction, so a failed unregister causes the mapping to be intentionally leaked rather than unmapped under a potentially live KVM slot.

Guest-memory read/write helpers validate guest address plus length against the registered region before any host pointer arithmetic. Flat-image loading uses those helpers; a guest physical address is never cast to a host pointer.

vCPU register ioctls use fixed-layout x86-64 UAPI structures. General registers are initialized from a fully zeroed structure plus explicit RIP/RFLAGS values. Special registers begin from the KVM-created vCPU reset state, after which the real-mode segment bases/selectors and CR0 mode bits required by the fixture are explicitly normalized.

Completed exits cross into VM policy only as typed `VcpuExit` values. The centralized `vmexit` dispatcher captures copied RIP/RFLAGS state and either returns a `VmExitReport` for HLT or a structured `VmExitError` for an unsupported reason. This boundary does not expose pointers into `kvm_run` or guest RAM.

## Not yet present

There is no guest virtual-address translation, external guest-image parser, device model, MMIO, port I/O, interrupt injection, virtqueue, disk backend, snapshot decoder, or guest-controlled device descriptor parsing in this revision. Unsupported exits are rejected with structured diagnostics; they are not yet serviced.
