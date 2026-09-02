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
              ├─ KVM_RUN
              └─ VcpuExit classification + register readback
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

The deterministic fixture consists only of `HLT` at guest physical address `0x1000`. ELF parsing and Linux boot conventions are intentionally absent.

## vCPU execution

The current fixture uses KVM's newly-created x86 vCPU architectural reset state as the starting special-register state, then explicitly normalizes CS/DS/ES/FS/GS/SS base and selector values to zero and clears CR0 protected-mode/paging enable bits. All general registers are then set from a zeroed `kvm_regs` value with RIP set to the entry point and RFLAGS bit 1 set as required by x86.

The current CS=0 fixture deliberately limits its real-mode RIP to `0xffff`. Broader real-mode segment addressing and protected/long-mode setup belong to later guest boot work.

`Vcpu::run_once` retries an interrupted host syscall, performs exactly one completed `KVM_RUN`, reads the exit reason from the tested prefix of `kvm_run`, and returns `VcpuExit::Hlt` or an explicit `Unhandled { reason }`. This is classification, not yet a centralized exit dispatcher.

## Ownership and lifetime

`KvmBackend` owns the `/dev/kvm` descriptor. `Vm` owns the VM descriptor and its optional registered guest RAM. `Vcpu` owns the vCPU descriptor and a `KvmRunMapping`. Rust ownership is used for normal cleanup; explicit KVM slot removal protects the guest-RAM lifetime boundary when independent vCPU descriptors exist.

## Error boundary

Errors are categorized as:

- `HostEnvironment`: host file/device/I/O failures, including named VM and vCPU ioctls;
- `KvmCapability`: incompatible API version, absent required extension, or invalid kernel-reported mapping size;
- `Configuration`: unsupported VMM configuration or current real-mode entry limits;
- `GuestMemory`: invalid guest ranges, reserved-range overlap, mapping failures, bounds violations, or KVM RAM-registration failures;
- `GuestImage`: malformed or overflowing flat-image descriptions.

Future centralized VM-exit, device, snapshot, and invariant categories will be added only when those responsibilities exist.

## Deliberate non-abstractions

There is no generic hypervisor backend trait yet. KVM is the only implementation, and an abstraction would not have a second consumer. The KVM-specific plumbing is nevertheless isolated so a later raw-VMX research backend would not require leaking ioctls into VM policy.

There is also no multi-region memory map yet. `GuestMemoryRegion::overlaps` exists to make range semantics explicit and tested, but the VM intentionally supports only slot 0 in this milestone.

## Next architectural milestone

The next bounded slice should centralize VM-exit handling and structured diagnostics around the now-proven execution path, without adding multiple device families. A deterministic test-port device should wait until that exit boundary is explicit and tested.
