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
       ↓
   vCPU creation
       ↓
      Vcpu
       ↓
  kvm_run mapping
```

The KVM UAPI details live in `src/kvm/sys.rs`. Higher layers call typed Rust methods and do not issue raw `ioctl` operations directly.

## Ownership and lifetime

`KvmBackend` owns the `/dev/kvm` descriptor. `Vm` owns the VM descriptor. `Vcpu` owns the vCPU descriptor and a `KvmRunMapping`. Rust drop order ensures the mapping and descriptor remain alive for the full `Vcpu` lifetime and are released when the object is dropped.

No guest RAM registration exists yet, so this revision does not make claims about guest-memory lifetime.

## Error boundary

Errors are categorized as:

- `HostEnvironment`: host file/device/I/O failures;
- `KvmCapability`: incompatible API version, absent required extension, or invalid kernel-reported mapping size;
- `Configuration`: unsupported VMM configuration.

Future guest-memory, guest-image, VM-exit, device, snapshot, and invariant categories will be added only when those responsibilities exist.

## Deliberate non-abstractions

There is no generic hypervisor backend trait yet. KVM is the only implementation, and an abstraction would not have a second consumer. The KVM-specific plumbing is nevertheless isolated so a later raw-VMX research backend would not require leaking ioctls into VM policy.

## Next architectural milestone

The next bounded slice should introduce one rigorously checked guest RAM region with explicit guest-physical address types and KVM memory registration. Guest execution should wait until that memory subsystem has boundary tests.
