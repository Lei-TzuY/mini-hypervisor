# Safety assumptions

## Trust model

The Linux KVM kernel interface and explicitly supplied host process configuration are trusted. Guest-controlled addresses, lengths, CPU state, port-I/O metadata, future MMIO requests, and future device requests are untrusted at userspace policy boundaries.

The current HLT and debug-port fixtures are repository-owned test inputs rather than arbitrary external guest content. `FlatGuestImage` nevertheless applies range and entry validation so later callers do not acquire an unchecked loading path by accident.

## Unsafe boundary in the current milestone

`src/kvm/sys.rs` contains raw `ioctl` calls. The call sites use constants and structures matching the stable x86-64 Linux KVM UAPI and convert `-1` into `std::io::Error` immediately.

Successful `KVM_CREATE_VM` and `KVM_CREATE_VCPU` results are file descriptors owned by the caller; they are immediately wrapped in `OwnedFd`.

Before any vCPU exists, the VM backend configures KVM's x86 identity-map page and TSS pages in the reserved range `0xfeff_c000..0xff00_0000`. The required KVM capabilities are checked before VM creation, and guest RAM registration rejects overlap with this range.

The vCPU run structure is mapped using the exact positive size returned by `KVM_GET_VCPU_MMAP_SIZE`. The mapping must be at least large enough for the tested x86 `KvmRunIoPrefix` before it is accepted. HLT reads only the exit reason from that prefix. Port-I/O handling additionally reads the tested I/O union fields.

For `KVM_EXIT_IO`, `data_offset` is treated only as an offset into the owned `kvm_run` mapping, never as a pointer. The vCPU layer checks conversion to `usize`, checked `size * count`, checked end-offset addition, and the final range against the mmap length before pointer arithmetic. OUT bytes are copied into an owned `Vec<u8>` before they cross into VM-exit policy or device code.

For `KVM_EXIT_IO_IN`, device policy returns owned response bytes rather than a pointer. Before copying those bytes into `kvm_run`, the vCPU layer re-checks that the current exit is port I/O, requires IN direction, recomputes the complete checked data range, and requires the response length to equal that range exactly. Only then does the unsafe copy target the validated region. OUT exits cannot be given an input response, and short or oversized responses are rejected.

KVM documents port-I/O operations as pending until userspace re-enters `KVM_RUN`. The dispatcher therefore services a request without taking a completed-operation register snapshot and returns `Continue`. For IN, the response buffer is populated before re-entry; KVM transfers it into guest architectural state when the following `KVM_RUN` completes the pending operation. The deterministic input fixture proves consumption by having guest code store AL into checked guest RAM and reading that RAM only after the later HLT exit.

`execution::run_vcpu_until_stopped` places a finite completed-exit budget around repeated `KVM_RUN` calls. The budget is checked before each run, and only a successfully completed VM exit consumes one unit. A zero budget therefore performs no guest run, while host-side `KVM_RUN` failures do not masquerade as completed guest exits.

When the configured budget is exhausted, the loop returns a structured `ExitBudgetExhausted` error with the vCPU id, configured limit, completed count, and last completed raw exit reason when available. Budget exhaustion is never converted into a terminal `VmExitReport`. If the final permitted exit was serviceable I/O, the userspace service may already be prepared, but without another permitted `KVM_RUN` the VMM does not claim that the pending KVM operation completed or snapshot registers as completed post-I/O state.

`VmExecutionResult` stores only owned Rust data: copied `PortIoExit` metadata/payloads, a terminal report, and a count. It contains no pointer or borrow into `kvm_run`, guest RAM, or a vCPU descriptor.

Guest RAM is a private anonymous mapping owned by `GuestMemory`. Region construction validates non-zero size, 4 KiB alignment, and guest-physical end arithmetic before `mmap` or KVM registration. After `KVM_SET_USER_MEMORY_REGION` succeeds, the `Vm` owns the mapping.

Before a `Vm` releases registered RAM, it explicitly removes KVM slot 0 with a zero-sized memory-region update. A separate vCPU fd may keep the kernel VM alive after the userspace VM handle begins destruction, so a failed unregister causes the mapping to be intentionally leaked rather than unmapped under a potentially live KVM slot.

Guest-memory read/write helpers validate guest address plus length against the registered region before any host pointer arithmetic. Flat-image loading uses those helpers; a guest physical address is never cast to a host pointer.

vCPU register ioctls use fixed-layout x86-64 UAPI structures. General registers are initialized from a fully zeroed structure plus explicit RIP/RFLAGS values. Special registers begin from the KVM-created vCPU reset state, after which the real-mode segment bases/selectors and CR0 mode bits required by the fixtures are explicitly normalized.

The minimal `PortIoBus` recognizes only debug port `0xe9`. The device accepts byte-wide, single-count accesses only. OUT requires exactly one copied payload byte; IN returns exactly one configured byte. Unknown ports, wide accesses, multi-count accesses, malformed OUT payloads, and malformed IN response lengths are explicit structured errors; no guest request is silently coerced.

## Not yet present

There is no guest virtual-address translation, external guest-image parser, MMIO, interrupt injection, explicit CPUID policy, virtqueue, disk backend, snapshot decoder, dynamic device registration, scheduler, or guest-controlled device descriptor parsing in this revision. Unsupported exits and unsupported port requests are rejected with structured diagnostics rather than serviced heuristically.
