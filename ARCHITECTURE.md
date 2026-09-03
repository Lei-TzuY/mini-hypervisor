# Architecture

## Current slice

```text
CLI
 ↓
VmConfig
 ↓
KvmBackend
 ├─ host capability validation
 ├─ bounded KVM_GET_SUPPORTED_CPUID
 │    └─ HostCpuid
 │         └─ GuestCpuPolicy::from_host
 ├─ bounded KVM_GET_MSR_INDEX_LIST
 │    └─ HostMsrIndexList
 ├─ bounded KVM_GET_MSR_FEATURE_INDEX_LIST
 │    └─ HostMsrFeatureIndexList
 │         └─ bounded system KVM_GET_MSRS
 │              └─ HostMsrFeatureValues
 │                   ├─ ModelImmutable
 │                   │    └─ HostMsrModelCandidate
 │                   └─ HostMutable (MSR_IA32_UCODE_REV)
 └─ VM creation
       ├─ x86 identity-map/TSS setup before vCPUs
       ↓
      Vm
       ├─ owns one registered GuestMemory mapping
       │       ↑
       │   FlatGuestImage
       │       └─ checked flat-binary load
       └─ vCPU creation
              ├─ KVM_CREATE_VCPU
              ├─ GuestCpuPolicy → KVM_SET_CPUID2
              ├─ bounded KVM_GET_CPUID2 → typed exact verification
              ↓
             Vcpu
              ├─ explicit real-mode register setup
              ├─ kvm_run mapping
              ├─ checked KVM_EXIT_IO metadata/payload extraction
              ├─ checked KVM_EXIT_IO_IN response write-back
              └─ KVM_RUN → VcpuExit
                         ↓
             execution::run_vcpu_until_stopped
              ├─ explicit completed-exit budget
              ├─ records serviced typed I/O exits
              └─ vmexit::dispatch_vcpu_exit
                   ├─ HLT → VmExitReport → stop
                   ├─ IO  → PortIoBus → debug port 0xe9 → continue
                   └─ other → VmExitError
```

The KVM UAPI details live in `src/kvm/sys.rs`. Higher layers call typed Rust methods and do not issue raw `ioctl` operations directly.

## x86 host and CPU capability contract

The backend requires KVM API version 12 plus `KVM_CAP_USER_MEMORY`, `KVM_CAP_SET_TSS_ADDR`, `KVM_CAP_EXT_CPUID`, `KVM_CAP_SET_IDENTITY_MAP_ADDR`, and `KVM_CAP_GET_MSR_FEATURES`.

After fixed host capability validation, `KvmBackend` performs `KVM_GET_SUPPORTED_CPUID` through a fixed 256-entry `repr(C)` `KvmCpuid2<N>` buffer. The header remains the exact 8-byte KVM ABI header and each trailing `KvmCpuidEntry2` is the exact 40-byte x86 entry. Pure tests lock header size, entry size, entry offset, and the supported/set/get CPUID ioctl request numbers.

The kernel-returned `nent` is not trusted as a Rust slice length. It must be non-zero and no greater than the fixed capacity before the entry prefix is converted into owned `CpuidEntry` values. Reserved KVM padding is intentionally absent from the typed representation, and conversion back to KVM UAPI always writes zero padding.

Host discovery and guest policy are separate states. `HostCpuid` preserves the validated KVM-supported function/index/flags/register values exactly; it contains no guest feature masking. `GuestCpuPolicy::from_host` clones that snapshot and applies the current no-LAPIC policy as a pure transformation. Pure tests require that construction not mutate the host snapshot, that unrelated metadata/registers are preserved, and that every matching indexed entry is handled consistently.

The current interrupt model has no in-kernel LAPIC or IRQ chip. Linux KVM documents x2APIC, TSC-deadline, and `KVM_FEATURE_PV_UNHALT` as depending on that model, so `GuestCpuPolicy` removes those three bits. No additional CPU feature is synthesized. This remains a host-derived runtime contract, not a migration-stable named CPU model.

`KvmBackend` retains both the discovered `HostCpuid` and derived `GuestCpuPolicy`. A `Vm` receives only the guest policy. `Vm::create_vcpu` performs `KVM_CREATE_VCPU`, serializes the policy into a fresh zero-padded KVM buffer, applies it with `KVM_SET_CPUID2`, then immediately reads the created vCPU contract back through a second fixed 256-entry buffer using `KVM_GET_CPUID2`. The read-back `nent` is independently validated before any slice is formed; entries are converted into owned `CpuidEntry` values and must exactly match the submitted policy in count, order, function, index, flags, and architectural registers. Only after that verification does the method construct/return `Vcpu`. SET, GET, count-validation, or comparison failures close the fresh descriptor through `OwnedFd` drop.

The deterministic CPUID fixture proves the configured contract from inside the guest rather than trusting host-side policy construction and read-back alone. In real mode it executes `CPUID(1)`, stores ECX at guest physical `0x2000`, executes `CPUID(0x40000001)`, stores EAX at `0x2004`, then halts. After the terminal exit, host code reads the checked eight-byte result range and exposes the observations through `CpuidGuestResult`. The integration regression requires x2APIC, TSC-deadline, and PV-unhalt to remain clear in guest-observed state.

### Host MSR capability snapshots

Both MSR-index discovery paths are system ioctls over the same variable-length `struct kvm_msr_list` ABI: a 4-byte `nmsrs` header followed by `u32` indices. Pure tests lock the header size, trailing-index offset, and the exact request values for `KVM_GET_MSR_INDEX_LIST` and `KVM_GET_MSR_FEATURE_INDEX_LIST`.

`KVM_GET_MSR_INDEX_LIST` describes the general MSR indices KVM exposes through its MSR access interface. Discovery is deliberately two-stage. First, `KvmBackend` submits a zero-capacity `KvmMsrList<0>` probe. KVM writes the required count back into `nmsrs` before returning `E2BIG` when the supplied capacity is insufficient, so `E2BIG` is accepted only for this probe. The reported count must be in `1..=1024`, where 1024 is this project's defensive bound rather than a claim about a Linux ABI maximum. The backend then issues a second query with a fixed 1024-entry `repr(C)` buffer and independently validates the final returned count before any Rust slice is formed.

`KVM_GET_MSR_FEATURE_INDEX_LIST` is a separate capability-discovery contract guarded by `KVM_CAP_GET_MSR_FEATURES`. It reports the feature-MSR indices that may be supplied to the system-level `KVM_GET_MSRS` interface for host/KVM feature probing. It uses the same zero-capacity probe and fixed 1024-entry second query, but its validated count is `0..=1024`: an empty feature list is a valid capability snapshot and is not treated as malformed. The second returned count is independently checked before slicing just like the general list.

Validated raw indices are copied into distinct owned typed snapshots. Both lists reuse `MsrIndex`, and both normalize repeated kernel indices by keeping the first occurrence while preserving the kernel's reported order for retained entries. `HostMsrIndexList` represents the general index set; `HostMsrFeatureIndexList` represents the feature-probing index set. Keeping these types separate prevents the two kernel contracts from becoming interchangeable merely because their wire representation is identical. Pure tests lock order preservation, duplicate normalization, typed reuse, and the valid empty feature-list case. Raw variable-length KVM buffers do not escape the KVM module.

The backend then reads the normalized feature indices through the system form of `KVM_GET_MSRS`. `KvmMsrs<N>` models the exact variable-length UAPI as an 8-byte `nmsrs`/padding header followed by 16-byte `KvmMsrEntry` values. Its constructor zeroes the header padding and every entry's reserved/data fields. Before the unsafe ioctl boundary, the wrapper independently requires the requested `nmsrs` to fit the actual backing array, so a malformed userspace header cannot authorize a kernel copy beyond the fixed Rust object.

`KVM_GET_MSRS` returns the number of entries successfully processed rather than rewriting `nmsrs` as a completion count. The backend therefore requires the returned count to equal the complete requested feature-index count. A partial result is rejected as malformed host-discovery state and identifies the first unread feature index when one exists; a returned count greater than requested is also rejected. Before any data becomes typed state, each returned entry index must still equal the requested index at the same position. Only a complete index-stable response becomes owned `MsrFeatureValue` entries inside `HostMsrFeatureValues`. An empty feature-index snapshot produces an empty value snapshot without issuing the value ioctl.

Every `MsrFeatureValue` now carries an `MsrFeatureStability` assigned inside its crate-private constructor, so callers cannot attach inconsistent stability metadata. Linux KVM treats feature MSRs as immutable once the vCPU model is defined except for `MSR_IA32_UCODE_REV`, which tracks the currently loaded microcode patch. The exact architectural index `0x8b` is therefore classified `HostMutable`; every other value returned through the current KVM feature-MSR contract is classified `ModelImmutable`. `HostMsrFeatureValues::model_immutable_values` and `host_mutable_values` expose order-preserving, mutually exclusive views over the same owned snapshot, and focused tests lock the `0x8b` constant plus partition behavior.

`ModelImmutable` is deliberately a narrow KVM-model statement, not a migration guarantee. It means KVM treats that feature MSR as immutable after the vCPU model is defined; it does not claim that two hosts, kernel versions, CPU revisions, or VMM configurations will expose the same value. `HostMutable` makes the microcode-revision exception explicit so it cannot be silently consumed as though it belonged to an immutable model-capability set.

`HostMsrFeatureValues::model_candidate` is the single public materialization path for `HostMsrModelCandidate`. The candidate owns a clone of the complete source observation as provenance and separately owns only the source entries classified `ModelImmutable`, preserving their original order and values. Candidate fields and its internal constructor are private, so callers cannot construct a candidate that injects `HostMutable` entries or mismatches the immutable subset with unrelated provenance. All-mutable and empty observations therefore produce valid empty candidate value sets while still retaining the complete source observation. Materialization is pure host-side Rust code and issues no additional KVM operation.

The model-candidate type remains intentionally weaker than a migration-stable CPU model. Its provenance records exactly which complete feature-value observation produced the immutable subset, but neither the candidate nor the `ModelImmutable` label claims equality across machines or software revisions. It is a typed input for later compatibility reasoning, not guest configuration.

The general-index, feature-index, and feature-value snapshots remain owned only by `KvmBackend` and are exposed read-only. Model candidates are materialized on demand from that read-only feature-value observation rather than duplicated as backend state. None is copied into `Vm` or `Vcpu`; no guest MSR allow/deny/value policy is derived from them; this slice issues no additional ioctl, no vCPU `KVM_GET_MSRS`, and no `KVM_SET_MSRS`.

## x86 VM setup

Immediately after `KVM_CREATE_VM`, before any vCPU can exist, the backend places the one-page identity-map region at `0xfeff_c000` and the three-page TSS region at `0xfeff_d000`. Together these reserve `0xfeff_c000..0xff00_0000`.

Those pages are intentionally outside the current low 2 MiB RAM fixture. Guest RAM registration rejects any region overlapping the reserved range so a future configurable RAM base cannot silently violate the x86 KVM requirement.

## Guest memory

`GuestPhysAddr` distinguishes guest physical addresses from host pointers. `GuestMemoryRegion` owns checked range semantics; `GuestMemory` owns the anonymous host mapping and performs guest-address validation before host memory copies. The current implementation accepts exactly one page-aligned, non-zero RAM region and registers it as KVM slot 0.

The region constructor rejects guest-physical wraparound and alignment errors. Access validation rejects address-plus-length overflow, ranges outside RAM, and host-size conversion failures. Zero-length accesses are valid at the exclusive end; non-zero accesses are not.

The `Vm` takes ownership of `GuestMemory` only after `KVM_SET_USER_MEMORY_REGION` succeeds. During `Vm` destruction it first issues a zero-sized slot-0 update to unregister RAM. If KVM refuses that cleanup, the process intentionally leaks the backing mapping rather than unmapping memory while a surviving vCPU fd could still keep the kernel VM alive.

See [docs/memory-map.md](docs/memory-map.md).

## Flat guest loading

`FlatGuestImage` is deliberately narrower than a general executable loader. Construction requires a non-empty byte slice, rejects load-address overflow, and requires the entry point to lie inside the loaded image. Loading still goes through `GuestMemory::write`, so a valid image description cannot escape the configured RAM region.

The HLT fixture contains only `HLT` at guest physical address `0x1000`. The port-output fixture contains `MOV AL, 'K'; OUT 0xe9, AL; HLT`. The port-input fixture contains `IN AL, 0xe9; MOV [0x2000], AL; HLT`. The CPUID-policy fixture is a reviewed 28-byte real-mode instruction stream that executes two CPUID leaves, stores two 32-bit observations at `0x2000..0x2008`, and halts. All use entry `0x1000`. ELF parsing and Linux boot conventions are intentionally absent.

## vCPU execution

The current fixtures use KVM's newly-created x86 vCPU architectural reset state as the starting special-register state, then explicitly normalize CS/DS/ES/FS/GS/SS base and selector values to zero and clear CR0 protected-mode/paging enable bits. All general registers are then set from a zeroed `kvm_regs` value with RIP set to the entry point and RFLAGS bit 1 set as required by x86.

The current CS=0 fixture deliberately limits its real-mode RIP to `0xffff`. Broader real-mode segment addressing and protected/long-mode setup belong to later guest boot work.

`Vcpu::run_once` retries an interrupted host syscall, performs one completed `KVM_RUN`, reads the tested x86 prefix of `kvm_run`, and returns a typed `VcpuExit`. HLT and I/O are classified explicitly; unknown reasons retain the exact raw reason.

For `KVM_EXIT_IO`, `Vcpu::port_io_exit` is the only layer allowed to inspect the I/O union and referenced data area. It validates direction, converts and checks `data_offset`, computes `size * count` with checked arithmetic, validates the complete range against the mmap length, and copies OUT bytes into owned Rust memory. IN requests expose metadata but no borrowed mmap data. No pointer into `kvm_run` leaves the vCPU layer.

For `KVM_EXIT_IO_IN`, `Vcpu::write_port_io_input` re-reads the current I/O metadata, requires IN direction, recomputes the checked mmap range, requires the device response length to match that range exactly, and only then copies the owned response bytes into `kvm_run`.

## Bounded execution loop

`execution::run_vcpu_until_stopped` is the single reusable run-loop boundary for the current one-vCPU model. Before each `KVM_RUN` it checks an explicit completed-exit budget. A successful `KVM_RUN` consumes exactly one budget unit; host-side failures that do not produce a completed VM exit consume none.

Each completed exit is sent through `vmexit::dispatch_vcpu_exit`. Serviceable I/O is recorded as an owned `PortIoExit` and execution continues while budget remains. A terminal HLT returns `VmExecutionResult`, which contains the terminal `VmExitReport`, every serviced typed I/O exit, and the exact completed-exit count.

A zero budget fails before any guest run. When the budget has been fully consumed, the next run attempt fails with `VmExitError::ExitBudgetExhausted`, preserving vCPU id, configured budget, completed count, and the last completed raw exit reason when available. Exhaustion is not reported as guest termination. If the final permitted exit was serviceable port I/O, userspace may have prepared the service response but the VMM does not claim the pending KVM operation completed because no further `KVM_RUN` was permitted.

The HLT and CPUID fixtures use budget 1. Both deterministic port fixtures use budget 2, so their successful sequence is exactly one serviceable I/O exit followed by terminal HLT. Extra serviceable exits cannot be silently accepted: they consume the budget and prevent a terminal success report.

## VM-exit dispatch

`vmexit::dispatch_vcpu_exit` is the single policy boundary for one completed vCPU exit.

- HLT snapshots RIP/RFLAGS and becomes `VmExitDisposition::Stopped(VmExitReport)`.
- Port I/O is parsed into an owned `PortIoExit` and routed through `PortIoBus`.
- An OUT service records/captures device output and becomes `Continue` without writing the run mapping.
- An IN service returns owned response bytes; the dispatcher asks the vCPU layer to validate and write those bytes into the pending KVM input range before returning `Continue`.
- Unsupported raw exit reasons become `VmExitError::Unhandled` with vCPU id and register diagnostics.

The dispatcher deliberately does **not** snapshot registers for an in-flight KVM I/O exit. KVM defines port-I/O operations as pending until userspace re-enters `KVM_RUN`; register state used as a completed-operation diagnostic is therefore taken only on a later terminal exit.

The deterministic output fixture reaches HLT at RIP `0x1005`. The deterministic input fixture receives byte `R`, re-enters KVM so KVM transfers that byte into AL, executes `MOV [0x2000], AL`, and reaches HLT at RIP `0x1006`. The CPUID fixture has no userspace-serviced exits and reaches HLT at RIP `0x101c`; host code then reads its two checked result words.

## Port-I/O bus and debug device

`PortIoBus` is intentionally minimal. It may contain one exact debug-port device at port `0xe9`; it is not yet a general dynamic device registry or port-range resolver.

The debug device accepts only byte-wide, single-count accesses at `0xe9`:

- OUT requires a copied payload length of exactly 1 byte and appends that byte to the device output buffer.
- IN returns exactly one configured byte as owned response data.

Unknown ports become `PortIoError::UnhandledPort`. Wider or multi-count operations to `0xe9` become `PortIoError::UnsupportedDebugAccess`. An OUT payload-length mismatch is explicit. The vCPU layer independently rejects an IN response whose length does not exactly match the checked KVM data range. No request is silently truncated, widened, repeated, or redirected.

## Ownership and lifetime

`KvmBackend` owns `/dev/kvm`, validated host capabilities, the typed `HostCpuid`, `HostMsrIndexList`, `HostMsrFeatureIndexList`, and stability-annotated `HostMsrFeatureValues` discovery snapshots, and the derived `GuestCpuPolicy`. `HostMsrModelCandidate` is an on-demand owned derivative: it clones the complete feature-value observation as provenance and separately owns only its immutable subset. `Vm` owns the VM descriptor, a clone of only the guest CPUID policy, and its optional registered guest RAM. CPUID read-back buffers and decoded comparison entries are temporary data inside vCPU construction; neither is retained after exact verification. `Vcpu` owns the vCPU descriptor and `KvmRunMapping`. `PortIoBus` owns its optional debug device, configured input byte, and accepted output bytes. `VmExecutionResult` and `CpuidGuestResult` own only copied safe Rust data and reports; neither contains a pointer or borrow into KVM shared memory or guest RAM.

Rust ownership is used for normal cleanup; explicit KVM slot removal protects the guest-RAM lifetime boundary when independent vCPU descriptors exist.

## Error boundary

Errors are categorized as:

- `HostEnvironment`: host file/device/I/O failures, malformed host KVM variable-length responses including CPUID, general MSR-index, MSR-feature-index, and MSR-feature-value completion/index semantics, named VM/vCPU ioctls including CPUID query/application/read-back, and CPUID read-back policy mismatches;
- `KvmCapability`: incompatible API version, absent required extension, or invalid kernel-reported mapping size;
- `Configuration`: unsupported VMM configuration or current real-mode entry limits;
- `GuestMemory`: invalid guest ranges, reserved-range overlap, mapping failures, bounds violations, or KVM RAM-registration failures;
- `GuestImage`: malformed or overflowing flat-image descriptions;
- `VmExit`: unsupported exits, malformed KVM I/O metadata/ranges, invalid IN response direction/length, execution-budget exhaustion, or deterministic fixture sequence failures;
- `PortIo`: unknown ports or unsupported/malformed device accesses.

Future MMIO, interrupt, snapshot, and stronger invariant categories will be added only when those responsibilities exist.

## Deliberate non-abstractions

There is no generic hypervisor backend trait yet. KVM is the only implementation, and an abstraction would not have a second consumer. The KVM-specific plumbing is nevertheless isolated so a later raw-VMX research backend would not require leaking ioctls into VM policy.

There is no configurable or migration-stable CPU model yet. The current boundary distinguishes discovered host support from a derived guest CPUID policy, but there is still exactly one built-in CPUID policy: host/KVM-supported entries with conservative masking for the absent LAPIC model. The vCPU creation path requires KVM to report back exactly that submitted typed CPUID policy, and the existing CPUID fixture independently verifies selected architectural bits from guest-observed state. `HostMsrIndexList`, `HostMsrFeatureIndexList`, stability-annotated `HostMsrFeatureValues`, and the derived `HostMsrModelCandidate` remain host-only state; neither `ModelImmutable` nor candidate membership is promoted into a cross-host stability guarantee, and there is still no guest MSR policy or guest MSR value lifecycle.

There is also no multi-region memory map yet. `GuestMemoryRegion::overlaps` exists to make range semantics explicit and tested, but the VM intentionally supports only slot 0 in this milestone.

The port bus is not a trait-object registry yet. One exact bidirectional device is enough to prove OUT copying and IN response write-back without introducing registration/range-resolution machinery prematurely.

The execution loop is not a scheduler. It owns no vCPU, thread, timer, or interrupt state; it only bounds repeated execution of one already-created vCPU.

## Next architectural milestone

The next bounded slice should add a pure host-only compatibility/diff contract between two `HostMsrModelCandidate` values. It should distinguish exact equality from missing indices, extra indices, and same-index value mismatches while preserving both candidates' provenance in diagnostics/results. The comparison must operate only on the immutable candidate values and must not consult or reintroduce `HostMutable` source entries. It should remain a compatibility-analysis primitive rather than declaring either candidate migration-safe, and it should not derive or apply guest MSR policy, issue vCPU `KVM_GET_MSRS`, call `KVM_SET_MSRS`, define a named migration-stable CPU model, or add long-mode boot, interrupts, MMIO, SMP, or device expansion in the same slice.
