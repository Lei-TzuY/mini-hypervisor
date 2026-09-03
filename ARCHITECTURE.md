# Architecture

## Current slice

```text
CLI
 ↓
VmConfig
 ↓
KvmBackend
 ├─ host capability validation
 └─ VM creation
       ├─ x86 identity-map/TSS setup before vCPUs
       ↓
      Vm
       ├─ owns one registered GuestMemory mapping
       │       ↑
       │   FlatGuestImage
       │       └─ checked flat-binary load
       └─ vCPU creation
              ↓
             Vcpu
              ├─ explicit real-mode register setup
              ├─ kvm_run mapping
              ├─ checked KVM_EXIT_IO metadata/payload extraction
              ├─ checked KVM_EXIT_IO_IN response write-back
              └─ KVM_RUN → VcpuExit
                         ↓
                 vmexit::dispatch_vcpu_exit
                  ├─ HLT → VmExitReport
                  ├─ IO  → PortIoBus → debug port 0xe9 → Continue
                  └─ other → VmExitError
```

The KVM UAPI details live in `src/kvm/sys.rs`. Higher layers call typed Rust methods and do not issue raw `ioctl` operations directly.

## x86 VM setup

The backend requires `KVM_CAP_SET_TSS_ADDR` and `KVM_CAP_SET_IDENTITY_MAP_ADDR` in addition to user-memory support. Immediately after `KVM_CREATE_VM`, before any vCPU can exist, it places the one-page identity-map region at `0xfeff_c000` and the three-page TSS region at `0xfeff_d000`. Together these reserve `0xfeff_c000..0xff00_0000`.

Those pages are intentionally outside the current low 2 MiB RAM fixture. Guest RAM registration rejects any region overlapping the reserved range so a future configurable RAM base cannot silently violate the x86 KVM requirement.

## Guest memory

`GuestPhysAddr` distinguishes guest physical addresses from host pointers. `GuestMemoryRegion` owns checked range semantics; `GuestMemory` owns the anonymous host mapping and performs guest-address validation before host memory copies. The current implementation accepts exactly one page-aligned, non-zero RAM region and registers it as KVM slot 0.

The region constructor rejects guest-physical wraparound and alignment errors. Access validation rejects address-plus-length overflow, ranges outside RAM, and host-size conversion failures. Zero-length accesses are valid at the exclusive end; non-zero accesses are not.

The `Vm` takes ownership of `GuestMemory` only after `KVM_SET_USER_MEMORY_REGION` succeeds. During `Vm` destruction it first issues a zero-sized slot-0 update to unregister RAM. If KVM refuses that cleanup, the process intentionally leaks the backing mapping rather than unmapping memory while a surviving vCPU fd could still keep the kernel VM alive.

See [docs/memory-map.md](docs/memory-map.md).

## Flat guest loading

`FlatGuestImage` is deliberately narrower than a general executable loader. Construction requires a non-empty byte slice, rejects load-address overflow, and requires the entry point to lie inside the loaded image. Loading still goes through `GuestMemory::write`, so a valid image description cannot escape the configured RAM region.

The HLT fixture contains only `HLT` at guest physical address `0x1000`. The port-output fixture contains `MOV AL, 'K'; OUT 0xe9, AL; HLT`. The port-input fixture contains `IN AL, 0xe9; MOV [0x2000], AL; HLT`. All use entry `0x1000`. ELF parsing and Linux boot conventions are intentionally absent.

## vCPU execution

The current fixtures use KVM's newly-created x86 vCPU architectural reset state as the starting special-register state, then explicitly normalize CS/DS/ES/FS/GS/SS base and selector values to zero and clear CR0 protected-mode/paging enable bits. All general registers are then set from a zeroed `kvm_regs` value with RIP set to the entry point and RFLAGS bit 1 set as required by x86.

The current CS=0 fixture deliberately limits its real-mode RIP to `0xffff`. Broader real-mode segment addressing and protected/long-mode setup belong to later guest boot work.

`Vcpu::run_once` retries an interrupted host syscall, performs one completed `KVM_RUN`, reads the tested x86 prefix of `kvm_run`, and returns a typed `VcpuExit`. HLT and I/O are classified explicitly; unknown reasons retain the exact raw reason.

For `KVM_EXIT_IO`, `Vcpu::port_io_exit` is the only layer allowed to inspect the I/O union and referenced data area. It validates direction, converts and checks `data_offset`, computes `size * count` with checked arithmetic, validates the complete range against the mmap length, and copies OUT bytes into owned Rust memory. IN requests expose metadata but no borrowed mmap data. No pointer into `kvm_run` leaves the vCPU layer.

For `KVM_EXIT_IO_IN`, `Vcpu::write_port_io_input` re-reads the current I/O metadata, requires IN direction, recomputes the checked mmap range, requires the device response length to match that range exactly, and only then copies the owned response bytes into `kvm_run`. A response can therefore neither target an OUT exit nor be truncated or overrun the mapped structure.

## VM-exit dispatch

`vmexit::dispatch_vcpu_exit` is the single policy boundary for vCPU exits.

- HLT snapshots RIP/RFLAGS and becomes `VmExitDisposition::Stopped(VmExitReport)`.
- Port I/O is parsed into an owned `PortIoExit` and routed through `PortIoBus`.
- An OUT service records/captures device output and becomes `Continue` without writing the run mapping.
- An IN service returns owned response bytes; the dispatcher asks the vCPU layer to validate and write those bytes into the pending KVM input range before returning `Continue`.
- Unsupported raw exit reasons become `VmExitError::Unhandled` with vCPU id and register diagnostics.

The dispatcher deliberately does **not** snapshot registers for an in-flight KVM I/O exit. KVM defines port-I/O operations as pending until userspace re-enters `KVM_RUN`; register state used as a completed-operation diagnostic is therefore taken only on a later terminal exit.

The deterministic output fixture services OUT, re-enters KVM, and reaches HLT at RIP `0x1005`. The deterministic input fixture services IN with byte `R`, re-enters KVM so KVM transfers that byte into AL, executes `MOV [0x2000], AL`, and reaches HLT at RIP `0x1006`. Host code then reads guest RAM at `0x2000` and requires that it contains `R`, proving the guest consumed the input rather than merely validating a host buffer.

## Port-I/O bus and debug device

`PortIoBus` is intentionally minimal. It may contain one exact debug-port device at port `0xe9`; it is not yet a general dynamic device registry or port-range resolver.

The debug device accepts only byte-wide, single-count accesses at `0xe9`:

- OUT requires a copied payload length of exactly 1 byte and appends that byte to the device output buffer.
- IN returns exactly one configured byte as owned response data.

Unknown ports become `PortIoError::UnhandledPort`. Wider or multi-count operations to `0xe9` become `PortIoError::UnsupportedDebugAccess`. An OUT payload-length mismatch is explicit. The vCPU layer independently rejects an IN response whose length does not exactly match the checked KVM data range. No request is silently truncated, widened, repeated, or redirected.

## Ownership and lifetime

`KvmBackend` owns the `/dev/kvm` descriptor. `Vm` owns the VM descriptor and its optional registered guest RAM. `Vcpu` owns the vCPU descriptor and a `KvmRunMapping`. `PortIoBus` owns its optional debug device, its configured input byte, and the output bytes that device has accepted. Rust ownership is used for normal cleanup; explicit KVM slot removal protects the guest-RAM lifetime boundary when independent vCPU descriptors exist.

## Error boundary

Errors are categorized as:

- `HostEnvironment`: host file/device/I/O failures, including named VM and vCPU ioctls;
- `KvmCapability`: incompatible API version, absent required extension, or invalid kernel-reported mapping size;
- `Configuration`: unsupported VMM configuration or current real-mode entry limits;
- `GuestMemory`: invalid guest ranges, reserved-range overlap, mapping failures, bounds violations, or KVM RAM-registration failures;
- `GuestImage`: malformed or overflowing flat-image descriptions;
- `VmExit`: unsupported exits, malformed KVM I/O metadata/ranges, invalid IN response direction/length, or deterministic fixture sequence failures;
- `PortIo`: unknown ports or unsupported/malformed device accesses.

Future MMIO, interrupt, snapshot, and stronger invariant categories will be added only when those responsibilities exist.

## Deliberate non-abstractions

There is no generic hypervisor backend trait yet. KVM is the only implementation, and an abstraction would not have a second consumer. The KVM-specific plumbing is nevertheless isolated so a later raw-VMX research backend would not require leaking ioctls into VM policy.

There is also no multi-region memory map yet. `GuestMemoryRegion::overlaps` exists to make range semantics explicit and tested, but the VM intentionally supports only slot 0 in this milestone.

The port bus is not a trait-object registry yet. One exact bidirectional device is enough to prove OUT copying and IN response write-back without introducing registration/range-resolution machinery prematurely.

## Next architectural milestone

The next bounded slice should replace fixture-specific two-step execution with one bounded vCPU execution loop that repeatedly runs, dispatches serviceable exits, and stops only on a terminal report or an explicit exit-budget limit. The loop should preserve the current pending-I/O completion semantics and return a structured error when the budget is exhausted. It should not add MMIO, interrupts, new devices, SMP, or scheduler machinery.
