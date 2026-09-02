# Architecture

## Current slice

```text
CLI
 ↓
VmConfig
 ↓
verify_kvm_lifecycle
 ↓
KvmBackend
 ├─ host capability validation
 └─ VM creation
       ↓
      Vm
       ├─ owns one registered GuestMemory mapping
       └─ vCPU creation
              ↓
             Vcpu
              ↓
         kvm_run mapping
```

The KVM UAPI details live in `src/kvm/sys.rs`. Higher layers call typed Rust methods and do not issue raw `ioctl` operations directly.

## Guest memory

`GuestPhysAddr` distinguishes guest physical addresses from host pointers. `GuestMemoryRegion` owns checked range semantics; `GuestMemory` owns the anonymous host mapping and performs guest-address validation before host memory copies. The current implementation accepts exactly one page-aligned, non-zero RAM region and registers it as KVM slot 0.

The region constructor rejects guest-physical wraparound and alignment errors. Access validation rejects address-plus-length overflow, ranges outside RAM, and host-size conversion failures. Zero-length accesses are valid at the exclusive end; non-zero accesses are not.

The `Vm` takes ownership of `GuestMemory` only after `KVM_SET_USER_MEMORY_REGION` succeeds. Its field order deliberately closes the VM descriptor before unmapping guest RAM during drop, so the host mapping remains alive through the registration lifetime.

See [docs/memory-map.md](docs/memory-map.md).

## Ownership and lifetime

`KvmBackend` owns the `/dev/kvm` descriptor. `Vm` owns the VM descriptor and its optional registered guest RAM. `Vcpu` owns the vCPU descriptor and a `KvmRunMapping`. Rust ownership is used for cleanup; raw KVM UAPI operations and mappings stay inside the backend/memory boundary.

## Error boundary

Errors are categorized as:

- `HostEnvironment`: host file/device/I/O failures;
- `KvmCapability`: incompatible API version, absent required extension, or invalid kernel-reported mapping size;
- `Configuration`: unsupported VMM configuration;
- `GuestMemory`: invalid guest ranges, mapping failures, bounds violations, or KVM RAM-registration failures.

Future guest-image, VM-exit, device, snapshot, and invariant categories will be added only when those responsibilities exist.

## Deliberate non-abstractions

There is no generic hypervisor backend trait yet. KVM is the only implementation, and an abstraction would not have a second consumer. The KVM-specific plumbing is nevertheless isolated so a later raw-VMX research backend would not require leaking ioctls into VM policy.

There is also no multi-region memory map yet. `GuestMemoryRegion::overlaps` exists to make range semantics explicit and tested, but the VM intentionally supports only slot 0 in this milestone.

## Next architectural milestone

The next bounded slice should load a tiny deterministic flat guest into the validated RAM region, initialize vCPU state explicitly, run it, and classify one expected exit. Device buses and richer exit dispatch should wait until that minimal execution path is proven.
