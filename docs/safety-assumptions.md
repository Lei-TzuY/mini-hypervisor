# Safety assumptions

## Trust model

The Linux KVM kernel interface and explicitly supplied host process configuration are trusted. Guest-controlled addresses, lengths, CPU state, port-I/O metadata, future MMIO requests, and future device requests are untrusted at userspace policy boundaries.

The current HLT and debug-port fixtures are repository-owned test inputs rather than arbitrary external guest content. `FlatGuestImage` nevertheless applies range and entry validation so later callers do not acquire an unchecked loading path by accident.

## Unsafe boundary in the current milestone

`src/kvm/sys.rs` contains raw `ioctl` calls. The call sites use constants and structures matching the stable x86-64 Linux KVM UAPI and convert `-1` into `std::io::Error` immediately.

Successful `KVM_CREATE_VM` and `KVM_CREATE_VCPU` results are file descriptors owned by the caller; they are immediately wrapped in `OwnedFd`.

Before any vCPU exists, the VM backend configures KVM's x86 identity-map page and TSS pages in the reserved range `0xfeff_c000..0xff00_0000`. The required KVM capabilities are checked before VM creation, and guest RAM registration rejects overlap with this range.

The vCPU run structure is mapped using the exact positive size returned by `KVM_GET_VCPU_MMAP_SIZE`. The mapping must be at least large enough for the tested x86 `KvmRunIoPrefix` before it is accepted. HLT reads only the exit reason from that prefix. Port-I/O handling additionally reads the tested I/O union fields.

For `KVM_EXIT_IO`, `data_offset` is treated only as an offset into the owned `kvm_run` mapping, never as a pointer. The vCPU layer checks conversion to `usize`, checked `size * count`, checked end-offset addition, and the final range against the mmap length before pointer arithmetic. OUT bytes are copied into an owned `Vec<u8>` before they cross into VM-exit policy or device code. IN data is not read or written in this milestone and is rejected by the only device.

KVM documents port-I/O operations as pending until userspace re-enters `KVM_RUN`. The dispatcher therefore services the copied request without taking a completed-operation register snapshot, returns `Continue`, and lets the execution loop re-enter KVM. The deterministic fixture takes its final register snapshot only after the following HLT exit.

Guest RAM is a private anonymous mapping owned by `GuestMemory`. Region construction validates non-zero size, 4 KiB alignment, and guest-physical end arithmetic before `mmap` or KVM registration. After `KVM_SET_USER_MEMORY_REGION` succeeds, the `Vm` owns the mapping.

Before a `Vm` releases registered RAM, it explicitly removes KVM slot 0 with a zero-sized memory-region update. A separate vCPU fd may keep the kernel VM alive after the userspace VM handle begins destruction, so a failed unregister causes the mapping to be intentionally leaked rather than unmapped under a potentially live KVM slot.

Guest-memory read/write helpers validate guest address plus length against the registered region before any host pointer arithmetic. Flat-image loading uses those helpers; a guest physical address is never cast to a host pointer.

vCPU register ioctls use fixed-layout x86-64 UAPI structures. General registers are initialized from a fully zeroed structure plus explicit RIP/RFLAGS values. Special registers begin from the KVM-created vCPU reset state, after which the real-mode segment bases/selectors and CR0 mode bits required by the fixtures are explicitly normalized.

The minimal `PortIoBus` recognizes only debug port `0xe9`. The device accepts one byte-wide, single-count OUT request with exactly one copied payload byte. Unknown ports, IN, wide accesses, multi-count accesses, and payload-length mismatches are explicit structured errors; no guest request is silently coerced.

## Not yet present

There is no guest virtual-address translation, external guest-image parser, MMIO, port input, interrupt injection, virtqueue, disk backend, snapshot decoder, dynamic device registration, or guest-controlled device descriptor parsing in this revision. Unsupported exits and unsupported port requests are rejected with structured diagnostics rather than serviced heuristically.
