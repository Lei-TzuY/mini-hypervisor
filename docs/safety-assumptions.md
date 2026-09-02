# Safety assumptions

## Trust model

The Linux KVM kernel interface and host process configuration are trusted. Future guest state and device requests are untrusted.

## Unsafe boundary in the current milestone

`src/kvm/sys.rs` contains raw `ioctl` calls. The call sites use constants from the stable Linux KVM UAPI and convert `-1` into `std::io::Error` immediately.

Successful `KVM_CREATE_VM` and `KVM_CREATE_VCPU` results are file descriptors owned by the caller; they are immediately wrapped in `OwnedFd`.

The vCPU run structure is mapped using the exact positive size returned by `KVM_GET_VCPU_MMAP_SIZE`. A successful non-null mapping is wrapped in `KvmRunMapping` and unmapped in `Drop`.

## Not yet present

There is no guest memory, guest pointer dereference, device model, MMIO, port I/O, virtqueue, disk backend, snapshot decoder, or guest-controlled length/address arithmetic in this revision.
