# Safety assumptions

## Trust model

The Linux KVM kernel interface and explicitly supplied host process configuration are trusted. Guest-controlled addresses, lengths, future CPU state, and future device requests are untrusted.

## Unsafe boundary in the current milestone

`src/kvm/sys.rs` contains raw `ioctl` calls. The call sites use constants from the stable x86-64 Linux KVM UAPI and convert `-1` into `std::io::Error` immediately.

Successful `KVM_CREATE_VM` and `KVM_CREATE_VCPU` results are file descriptors owned by the caller; they are immediately wrapped in `OwnedFd`.

The vCPU run structure is mapped using the exact positive size returned by `KVM_GET_VCPU_MMAP_SIZE`. A successful non-null mapping is wrapped in `KvmRunMapping` and unmapped in `Drop`.

Guest RAM is a private anonymous mapping owned by `GuestMemory`. Region construction validates non-zero size, 4 KiB alignment, and guest-physical end arithmetic before `mmap` or KVM registration. After `KVM_SET_USER_MEMORY_REGION` succeeds, the `Vm` owns the mapping.

Before a `Vm` releases registered RAM, it explicitly removes KVM slot 0 with a zero-sized memory-region update. A separate vCPU fd may keep the kernel VM alive after the userspace VM handle begins destruction, so a failed unregister causes the mapping to be intentionally leaked rather than unmapped under a potentially live KVM slot.

Guest-memory read/write helpers validate guest address plus length against the registered region before any host pointer arithmetic. The code never treats a guest physical address as a host pointer.

## Not yet present

There is no guest execution, guest virtual-address translation, device model, MMIO, port I/O, virtqueue, disk backend, snapshot decoder, or guest-controlled device descriptor parsing in this revision.
