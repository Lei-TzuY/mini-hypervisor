# Safety assumptions

## Trust model

The Linux KVM kernel interface and explicitly supplied host process configuration are trusted. Guest-controlled addresses, lengths, CPU state, port-I/O metadata, future MMIO requests, and future device requests are untrusted at userspace policy boundaries. Kernel-returned variable-length metadata such as supported-CPUID, vCPU CPUID read-back, and MSR-index counts is also validated before it is used for Rust slicing or copied into typed state.

The current HLT, debug-port, and CPUID fixtures are repository-owned test inputs rather than arbitrary external guest content. `FlatGuestImage` nevertheless applies range and entry validation so later callers do not acquire an unchecked loading path by accident.

## Unsafe boundary in the current milestone

`src/kvm/sys.rs` contains raw `ioctl` calls. The call sites use constants and structures matching the stable x86-64 Linux KVM UAPI and convert `-1` into `std::io::Error` immediately.

Successful `KVM_CREATE_VM` and `KVM_CREATE_VCPU` results are file descriptors owned by the caller; they are immediately wrapped in `OwnedFd`.

### CPUID contract

`KVM_GET_SUPPORTED_CPUID`, `KVM_SET_CPUID2`, and `KVM_GET_CPUID2` use the variable-length `struct kvm_cpuid2` ABI. The implementation does not allocate an unaligned byte buffer and cast it. Instead, `KvmCpuid2<N>` is one `repr(C)` object containing the 8-byte header followed immediately by an aligned fixed array of `KvmCpuidEntry2` values. Pure tests lock the 40-byte entry size, header size, entries offset, and all three ioctl request values.

Both the system discovery query and vCPU read-back use a fixed capacity of 256 entries, matching the current x86 KVM maximum used by this project. Each successful kernel-returned `nent` is treated as untrusted metadata: it must be in `1..=256` before a slice is formed. A zero or out-of-capacity count is rejected as an invalid host/KVM response. Capacity failure from the ioctl itself remains a named `KVM_GET_CPUID2` vCPU-operation error rather than being retried with an unbounded allocation.

After count validation, raw KVM entries are converted into owned `CpuidEntry` values that contain only function, index, flags, and architectural register values. Reserved KVM padding is not retained in typed host or read-back state. Converting a final guest policy back into a KVM buffer always recreates the reserved padding as zero. Raw variable-length buffers never cross above the KVM module boundary.

Host capability discovery and guest policy construction are separate trust boundaries. `HostCpuid` preserves the validated KVM-supported entry values and is never mutated by policy construction. `GuestCpuPolicy::from_host` clones that snapshot and performs only the current conservative feature reduction. Pure tests require host immutability, exact preservation of unrelated fields/registers, and consistent treatment of multiple indexed entries for the same function.

The current VMM intentionally has no in-kernel LAPIC/IRQ-chip model. Linux KVM documents x2APIC, TSC-deadline exposure, and `KVM_FEATURE_PV_UNHALT` as depending on that interrupt model, so those bits are removed only in `GuestCpuPolicy`. This is a conservative feature reduction; no unsupported feature is synthesized.

Every `Vm::create_vcpu` call serializes the already-derived guest policy into a fresh bounded KVM buffer and applies it with `KVM_SET_CPUID2`. Before the `Vcpu` object is returned, the same descriptor is queried with bounded `KVM_GET_CPUID2`. The returned count is independently validated, entries are decoded into owned typed values, and the complete ordered list must exactly equal the submitted `GuestCpuPolicy` across function, index, flags, EAX, EBX, ECX, and EDX. A SET failure, GET failure, malformed count, count mismatch, or field mismatch closes the newly created descriptor through `OwnedFd` drop and returns a named vCPU-operation error. Higher layers therefore cannot reach this project's `KVM_RUN` path through a vCPU whose configured CPUID contract was not verified after submission.

The deterministic CPUID proof executes only two fixed leaves. Its reviewed 28-byte real-mode program stores CPUID(1).ECX at guest physical `0x2000` and CPUID(0x40000001).EAX at `0x2004`, then halts. The host does not inspect either result until the terminal HLT report has been produced. The entire eight-byte result range is read through `GuestMemory::read`, so guest-written addresses still pass the normal checked guest-memory boundary before they become host observations. Pure tests separately lock the byte sequence, little-endian decoding, and the predicate used to identify the three masked LAPIC-dependent bits.

### MSR index capability discovery

`KVM_GET_MSR_INDEX_LIST` uses the variable-length `struct kvm_msr_list` ABI: a 4-byte `nmsrs` field followed by a trailing array of `u32` indices. `KvmMsrList<N>` models this as one aligned `repr(C)` object instead of casting an arbitrary byte buffer. Pure tests lock the zero-entry and one-entry sizes, the trailing-array offset, and the exact ioctl request value.

Discovery uses two bounded queries. The first query supplies `KvmMsrList<0>`. Linux KVM writes the required count back to `nmsrs` before returning `E2BIG` when the supplied capacity is too small, so `E2BIG` is accepted as expected only at this probe boundary; any other probe error remains a host-I/O failure. The returned required count must be in `1..=1024`. The 1024 limit is an explicit project defensive bound, not an assertion that Linux permanently caps this ABI at 1024 entries.

Only after the probe count passes validation does the backend issue the second query with a fixed `KvmMsrList<1024>`. The second returned `nmsrs` is independently validated in `1..=1024` before indexing the trailing array. A second-query failure, zero count, or count beyond the project bound is rejected rather than triggering unbounded allocation or unchecked slicing.

Validated raw indices are copied into owned `MsrIndex` values inside `HostMsrIndexList`. Duplicate kernel indices are normalized by retaining the first occurrence while preserving the kernel-reported order of retained indices; pure tests lock both behaviors. The raw variable-length buffer and its pointer never leave the KVM module.

This snapshot is capability discovery only. It is retained by `KvmBackend`, is not copied into `Vm` or `Vcpu`, and does not itself authorize guest MSR access. This milestone issues neither `KVM_GET_MSRS` nor `KVM_SET_MSRS`, reads no guest MSR value, writes no guest MSR value, and derives no guest MSR allow/deny policy.

### VM and memory setup

Before any vCPU exists, the VM backend configures KVM's x86 identity-map page and TSS pages in the reserved range `0xfeff_c000..0xff00_0000`. The required KVM capabilities are checked before VM creation, and guest RAM registration rejects overlap with this range.

The vCPU run structure is mapped using the exact positive size returned by `KVM_GET_VCPU_MMAP_SIZE`. The mapping must be at least large enough for the tested x86 `KvmRunIoPrefix` before it is accepted. HLT reads only the exit reason from that prefix. Port-I/O handling additionally reads the tested I/O union fields.

For `KVM_EXIT_IO`, `data_offset` is treated only as an offset into the owned `kvm_run` mapping, never as a pointer. The vCPU layer checks conversion to `usize`, checked `size * count`, checked end-offset addition, and the final range against the mmap length before pointer arithmetic. OUT bytes are copied into an owned `Vec<u8>` before they cross into VM-exit policy or device code.

For `KVM_EXIT_IO_IN`, device policy returns owned response bytes rather than a pointer. Before copying those bytes into `kvm_run`, the vCPU layer re-checks that the current exit is port I/O, requires IN direction, recomputes the complete checked data range, and requires the response length to equal that range exactly. Only then does the unsafe copy target the validated region. OUT exits cannot be given an input response, and short or oversized responses are rejected.

KVM documents port-I/O operations as pending until userspace re-enters `KVM_RUN`. The dispatcher therefore services a request without taking a completed-operation register snapshot and returns `Continue`. For IN, the response buffer is populated before re-entry; KVM transfers it into guest architectural state when the following `KVM_RUN` completes the pending operation. The deterministic input fixture proves consumption by having guest code store AL into checked guest RAM and reading that RAM only after the later HLT exit.

`execution::run_vcpu_until_stopped` places a finite completed-exit budget around repeated `KVM_RUN` calls. The budget is checked before each run, and only a successfully completed VM exit consumes one unit. A zero budget therefore performs no guest run, while host-side `KVM_RUN` failures do not masquerade as completed guest exits.

When the configured budget is exhausted, the loop returns a structured `ExitBudgetExhausted` error with the vCPU id, configured limit, completed count, and last completed raw exit reason when available. Budget exhaustion is never converted into a terminal `VmExitReport`. If the final permitted exit was serviceable I/O, the userspace service may already be prepared, but without another permitted `KVM_RUN` the VMM does not claim that the pending KVM operation completed or snapshot registers as completed post-I/O state.

`VmExecutionResult` stores only owned Rust data: copied `PortIoExit` metadata/payloads, a terminal report, and a count. `CpuidGuestResult` likewise stores only copied 32-bit observations and a terminal report. Neither contains a pointer or borrow into `kvm_run`, guest RAM, or a vCPU descriptor.

Guest RAM is a private anonymous mapping owned by `GuestMemory`. Region construction validates non-zero size, 4 KiB alignment, and guest-physical end arithmetic before `mmap` or KVM registration. After `KVM_SET_USER_MEMORY_REGION` succeeds, the `Vm` owns the mapping.

Before a `Vm` releases registered RAM, it explicitly removes KVM slot 0 with a zero-sized memory-region update. A separate vCPU fd may keep the kernel VM alive after the userspace VM handle begins destruction, so a failed unregister causes the mapping to be intentionally leaked rather than unmapped under a potentially live KVM slot.

Guest-memory read/write helpers validate guest address plus length against the registered region before any host pointer arithmetic. Flat-image loading uses those helpers; a guest physical address is never cast to a host pointer.

vCPU register ioctls use fixed-layout x86-64 UAPI structures. General registers are initialized from a fully zeroed structure plus explicit RIP/RFLAGS values. Special registers begin from the KVM-created vCPU reset state, after which the real-mode segment bases/selectors and CR0 mode bits required by the fixtures are explicitly normalized.

The minimal `PortIoBus` recognizes only debug port `0xe9`. The device accepts byte-wide, single-count accesses only. OUT requires exactly one copied payload byte; IN returns exactly one configured byte. Unknown ports, wide accesses, multi-count accesses, malformed OUT payloads, and malformed IN response lengths are explicit structured errors; no guest request is silently coerced.

## Not yet present

There is no guest virtual-address translation, external guest-image parser, MMIO, interrupt injection, configurable/migratable CPU model, guest MSR policy or guest MSR value lifecycle, virtqueue, disk backend, snapshot decoder, dynamic device registration, scheduler, or guest-controlled device descriptor parsing in this revision. The backend does have a bounded typed host MSR-index capability snapshot, but it does not confer guest MSR semantics. The current CPUID contract is deliberately host-derived and conservatively masked rather than a stable cross-host CPU profile. Unsupported exits and unsupported port requests are rejected with structured diagnostics rather than serviced heuristically.
