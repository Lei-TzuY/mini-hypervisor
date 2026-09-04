# Architecture

## Current slice

```text
CLI
 ↓
VmConfig
 ↓
KvmBackend
 ├─ required host capability validation + optional KVM_CAP_INTERNAL_ERROR_DATA observation
 ├─ bounded KVM_GET_SUPPORTED_CPUID
 │    └─ HostCpuid
 │         └─ GuestCpuPolicy::from_host
 │              └─ GuestCpuPolicyComparison
 ├─ bounded KVM_GET_MSR_INDEX_LIST
 │    └─ HostMsrIndexList
 │         └─ explicit MsrIndex[] → GuestMsrAccessPolicy (pure)
 │              └─ explicit (MsrIndex, u64)[] → GuestMsrValueSet (pure)
 ├─ bounded KVM_GET_MSR_FEATURE_INDEX_LIST
 │    └─ HostMsrFeatureIndexList
 │         └─ bounded system KVM_GET_MSRS
 │              └─ HostMsrFeatureValues
 │                   ├─ ModelImmutable
 │                   │    └─ HostMsrModelCandidate
 │                   │         └─ HostMsrModelComparison
 │                   └─ HostMutable (MSR_IA32_UCODE_REV)
 ├─ GuestCpuPolicy + HostMsrModelCandidate
 │    └─ CpuModelCandidate
 │         └─ CpuModelComparison
 └─ VM creation
       ├─ x86 identity-map/TSS setup before vCPUs
       ↓
      Vm
       ├─ owns one registered GuestMemory mapping
       │       ↑
       │   FlatGuestImage
       │       └─ checked flat-binary load
       │   LongModeBootLayout
       │       └─ checked fixed 2 MiB identity-map page tables
       └─ vCPU creation
              ├─ KVM_CREATE_VCPU
              ├─ GuestCpuPolicy → KVM_SET_CPUID2
              ├─ bounded KVM_GET_CPUID2 → typed exact verification
              ├─ propagate optional internal-error-data support
              ↓
             Vcpu
              ├─ explicit real-mode register setup for legacy fixtures
              ├─ explicit x86-64 long-mode sregs/regs setup for the long-mode fixture
              ├─ KVM_GET_REGS → VcpuRegisterSnapshot
              │    ├─ compare → VcpuRegisterSnapshotComparison (pure)
              │    ├─ verify_register_snapshot → fresh read-only capture
              │    ├─ restore_register_snapshot → KVM_SET_REGS
              │    └─ restore_and_verify_register_snapshot
              ├─ KVM_GET_SREGS → VcpuSpecialRegisterSnapshot
              │    ├─ compare → VcpuSpecialRegisterSnapshotComparison (pure)
              │    ├─ verify_special_register_snapshot → fresh read-only capture
              │    ├─ restore_special_register_snapshot → KVM_SET_SREGS
              │    └─ restore_and_verify_special_register_snapshot
              ├─ explicit MsrIndex[] → bounded KVM_GET_MSRS → VcpuMsrValues
              ├─ GuestMsrAccessPolicy → capture_msrs → GuestMsrValueSet
              ├─ GuestMsrAccessPolicy → capture_msr_snapshot → GuestMsrSnapshot
              ├─ GuestMsrSnapshot → compare → GuestMsrSnapshotComparison (pure)
              ├─ GuestMsrSnapshot → verify_msr_snapshot → fresh policy-bound read-only capture
              ├─ GuestMsrValueSet → bounded KVM_SET_MSRS
              ├─ GuestMsrSnapshot → restore_msr_snapshot → bounded KVM_SET_MSRS
              ├─ GuestMsrSnapshot → restore_and_verify_msr_snapshot → GuestMsrSnapshotComparison
              ├─ capture_state_snapshot → VcpuStateSnapshot
              │    ├─ compare → VcpuStateSnapshotComparison (pure)
              │    ├─ verify_state_snapshot → fresh read-only capture
              │    ├─ restore_state_snapshot → bounded non-transactional component restore
              │    └─ restore_and_verify_state_snapshot
              ├─ kvm_run mapping
              ├─ checked KVM_EXIT_IO metadata/payload extraction
              ├─ checked KVM_EXIT_IO_IN response write-back
              ├─ checked KVM_EXIT_UNKNOWN hardware-diagnostic extraction
              ├─ checked KVM_EXIT_EXCEPTION payload extraction
              ├─ checked KVM_EXIT_FAIL_ENTRY payload extraction
              ├─ capability-gated KVM_EXIT_INTERNAL_ERROR payload extraction
              │    └─ raw suberror → VcpuInternalErrorSuberror (pure, lossless classification)
              ├─ checked KVM_EXIT_SYSTEM_EVENT payload extraction
              └─ KVM_RUN → VcpuExit
                         ↓
             execution::run_vcpu_until_stopped
              ├─ explicit completed-exit budget
              ├─ ordered completed-exit reason trace
              ├─ records serviced typed I/O exits
              └─ vmexit::dispatch_vcpu_exit
                   ├─ HLT / legacy shutdown → VmExitReport → stop
                   ├─ IO → PortIoBus → debug port 0xe9 → continue
                   ├─ KVM_UNKNOWN → structured hardware diagnostic
                   ├─ EXCEPTION → structured exception diagnostic
                   ├─ FAIL_ENTRY → structured entry-failure diagnostic
                   ├─ INTERNAL_ERROR → structured capability-aware diagnostic
                   ├─ SYSTEM_EVENT → structured unsupported diagnostic
                   └─ other unsupported raw reason → VmExitError::Unhandled
```

The raw ioctl UAPI details live in `src/kvm/sys.rs`; tested `kvm_run` payload views stay isolated below the vCPU layer. Higher layers call typed Rust methods and do not issue raw `ioctl` operations or inspect raw shared-memory payload layouts directly.

## x86 host and CPU capability contract

The backend requires KVM API version 12 plus `KVM_CAP_USER_MEMORY`, `KVM_CAP_SET_TSS_ADDR`, `KVM_CAP_EXT_CPUID`, `KVM_CAP_SET_IDENTITY_MAP_ADDR`, and `KVM_CAP_GET_MSR_FEATURES`.

Separately from that required set, `KvmBackend` queries `KVM_CAP_INTERNAL_ERROR_DATA` (capability ID 40) through the same `KVM_CHECK_EXTENSION` boundary and stores the raw returned value in the existing owned `HostCapabilities.extensions` snapshot. A missing observation in manually constructed `HostCapabilities` or a recorded value of `0` does not invalidate an otherwise valid host; `internal_error_data_capability()` exposes the recorded observation when present and `supports_internal_error_data()` is true exactly when its raw value is greater than zero. `KvmBackend::create_vm` propagates only that support boolean into `Vm`, and each created `Vcpu` inherits the same fact. A non-positive observation keeps internal-error decoding on the always-available base `suberror` view. A positive observation authorizes the vCPU decoder to form the fixed full x86 internal-error payload view and validate/copy declared optional data; it still does not create emulation recovery, retry, replacement execution, or lifecycle policy. Typed suberror classification is independent of this capability.

After fixed host capability validation, `KvmBackend` performs `KVM_GET_SUPPORTED_CPUID` through a fixed 256-entry `repr(C)` `KvmCpuid2<N>` buffer. The kernel-returned `nent` is not trusted as a Rust slice length: it must be non-zero and no greater than the fixed capacity before the entry prefix is converted into owned `CpuidEntry` values. Reserved KVM padding is absent from the typed representation, and conversion back to KVM UAPI always writes zero padding.

Host discovery and guest policy are separate states. `HostCpuid` preserves the validated KVM-supported function/index/flags/register values exactly; `GuestCpuPolicy::from_host` clones that snapshot and applies the current no-LAPIC policy as a pure transformation. The current interrupt model has no in-kernel LAPIC or IRQ chip, so the guest policy removes x2APIC, TSC-deadline, and `KVM_FEATURE_PV_UNHALT` and synthesizes no additional feature.

`GuestCpuPolicy::compare` is a pure compatibility/diff primitive over two configured policies keyed by `(function,index)`. Reordered entries with identical keys/fields remain an exact match. Every comparison owns both source policies and directional findings. An exact comparison is not a migration-safety decision.

`Vm::create_vcpu` performs `KVM_CREATE_VCPU`, serializes the configured policy into a fresh zero-padded KVM buffer, applies it through `KVM_SET_CPUID2`, reads it back through bounded `KVM_GET_CPUID2`, and requires the complete returned list to match before publishing `Vcpu`.

The deterministic CPUID fixture proves selected configured bits from inside the guest. It remains a reviewed real-mode flat-binary fixture and is independent of the long-mode bootstrap.

### Host and guest MSR boundaries

The repository keeps general MSR-index discovery, feature-MSR discovery, feature-value stability, guest access policy, guest value sets, full guest snapshots, readback, writes, restore, verification, and CPU-model-candidate composition as separate typed boundaries.

Both variable-length MSR index-list paths use bounded `repr(C)` buffers and validate returned counts before slicing. System feature-MSR values require exact processed counts and exact index order before they become owned `HostMsrFeatureValues`. `MSR_IA32_UCODE_REV` is classified `HostMutable`; other values in the current KVM feature-MSR contract are `ModelImmutable`. `HostMsrModelCandidate` owns its complete source observation and only its immutable candidate values. Candidate comparisons are pure and are not migration guarantees.

`GuestMsrAccessPolicy::from_host` validates explicit caller-selected indices against the general host MSR-index snapshot. `GuestMsrValueSet::from_policy` validates explicit caller state against that policy. `GuestMsrSnapshot` is stronger than a general subset value set: it requires complete policy coverage and exact order. `Vcpu::msrs`, `capture_msrs`, `capture_msr_snapshot`, `verify_msr_snapshot`, `set_msrs`, `restore_msr_snapshot`, and `restore_and_verify_msr_snapshot` reuse bounded KVM request objects and preserve exact processed-count semantics. KVM short writes are explicitly non-transactional and may have changed the successful prefix.

`CpuModelCandidate` composes the configured guest CPUID policy and immutable host MSR candidate without creating a named migration-safe CPU model. `CpuModelComparison` delegates to the two component comparison contracts and retains their provenance.

## x86 VM setup

Immediately after `KVM_CREATE_VM`, before any vCPU can exist, the backend places the one-page KVM identity-map region at `0xfeff_c000` and the three-page TSS region at `0xfeff_d000`. Together these reserve `0xfeff_c000..0xff00_0000`.

Those KVM-reserved pages are distinct from the guest-created long-mode page tables at low guest physical addresses. Guest RAM registration rejects any region overlapping the high KVM-reserved range.

## Guest memory

`GuestPhysAddr` distinguishes guest physical addresses from host pointers. `GuestMemoryRegion` owns checked range semantics; `GuestMemory` owns the anonymous host mapping and performs guest-address validation before host memory copies. The current implementation accepts exactly one page-aligned, non-zero RAM region and registers it as KVM slot 0.

The region constructor rejects guest-physical wraparound and alignment errors. Access validation rejects address-plus-length overflow, ranges outside RAM, and host-size conversion failures. Zero-length accesses are valid at the exclusive end; non-zero accesses are not.

The `Vm` takes ownership of `GuestMemory` only after `KVM_SET_USER_MEMORY_REGION` succeeds. During `Vm` destruction it first issues a zero-sized slot-0 update to unregister RAM. If KVM refuses that cleanup, the process intentionally leaks the backing mapping rather than unmapping memory while a surviving vCPU fd could still keep the kernel VM alive.

See [docs/memory-map.md](docs/memory-map.md).

## Flat guest loading

`FlatGuestImage` is deliberately narrower than a general executable loader. Construction requires a non-empty byte slice, rejects load-address overflow, and requires the entry point to lie inside the loaded image. Loading still goes through `GuestMemory::write`, so a valid image description cannot escape the configured RAM region.

The existing HLT, debug-port output, debug-port input, and CPUID fixtures remain reviewed real-mode flat binaries at entry `0x1000`. The long-mode fixture is also a flat binary: it is loaded at GPA/VA `0x10000` only after the fixed bootstrap page tables have been installed. ELF parsing, relocation, Linux boot conventions, and a general executable format remain absent.

## x86-64 long-mode bootstrap

`LongModeBootLayout` is the single correctness contract for the current 64-bit bootstrap. It is intentionally fixed rather than a general virtual-memory manager.

The layout requires guest RAM to start at GPA `0` and contain at least `0x20_0000` bytes. Three 4 KiB pages are reserved for bootstrap translation state:

- PML4 at GPA `0x1000`;
- PDPT at GPA `0x2000`;
- PD at GPA `0x3000`;
- reserved page-table extent `0x1000..0x4000`.

`install_page_tables` zeroes all three pages through checked `GuestMemory::write`, then installs exactly one chain: PML4[0] = `0x2003`, PDPT[0] = `0x3003`, and PD[0] = `0x83`. The PD entry is a present, writable 2 MiB large page, so virtual addresses `0..0x20_0000` identity-map to the same guest physical addresses. No other mapping is created.

`LongModeBootLayout::new` rejects RAM with a non-zero base, RAM below 2 MiB, an entry at or beyond the identity-map extent, an entry inside the reserved page-table pages, a zero or out-of-map stack pointer, and a stack pointer that overlaps the bootstrap page-table region. The deterministic fixture uses entry GPA/VA `0x10000` and RSP `0x1ff000`.

`Vcpu::initialize_long_mode` begins from KVM's current special-register state. It preserves unrelated inherited control/EFER bits while requiring `CR0.PE|CR0.PG`, `CR4.PAE`, and `EFER.LME|EFER.LMA`; it writes `CR3 = 0x1000`. CS is a present ring-0 flat 64-bit code segment with selector `0x8`, long bit set, default operand-size bit clear, base zero, and limit `0xffff_ffff`. DS/ES/FS/GS/SS use the fixed present ring-0 data-segment contract with selector `0x10`, base zero, and limit `0xffff_ffff`. The general-register write sets RIP from the validated entry, RSP from the validated stack pointer, and RFLAGS bit 1 while zero-initializing the remaining general-register fields.

The deterministic 36-byte guest intentionally contains 64-bit-only/64-bit-width instruction encodings (`REX.W` `movabs` and 64-bit shifts). It emits `L`, `M`, `6`, `4` through four byte-wide OUT operations to the existing debug port `0xe9`, then executes HLT. The bounded run therefore completes exactly five exits: four I/O exits followed by HLT. A successful terminal report has RIP `0x10024` and the collected debug output is exactly `LM64`.

This contract proves deterministic x86-64 long-mode execution only. It does not create ELF loading, Linux boot, dynamic page-table construction, arbitrary virtual mappings, MMIO, APIC/interrupt infrastructure, virtio, SMP, migration, snapshots, or resumable execution.

## vCPU execution

The legacy real-mode fixtures start from KVM's new-vCPU reset state, normalize CS/DS/ES/FS/GS/SS base/selectors to zero, clear CR0 protected-mode/paging enable bits, then set a zeroed `kvm_regs` with RIP and architectural RFLAGS bit 1. Their CS=0 entry remains deliberately limited to `0xffff`.

The long-mode fixture follows the separate `LongModeBootLayout` and `Vcpu::initialize_long_mode` contract above. It does not transit through a guest-side real-to-protected-to-long-mode boot stub; userspace establishes the architectural long-mode state through KVM sregs/regs before the first `KVM_RUN`.

`Vcpu::capture_register_snapshot` performs one existing `KVM_GET_REGS` and copies all 18 x86 general-register fields into an owned `VcpuRegisterSnapshot`. Pure comparison, read-only verification, snapshot-bound restore, and restore-and-verify remain unchanged by the long-mode bootstrap.

Special-register capture likewise owns semantic x86 segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM padding. Pure comparison and read-only verification remain separate from restore. `VcpuStateSnapshot` composes general-register, special-register, and policy-bound MSR snapshots with explicitly bounded non-transactional semantics. None of these values is a whole-VM, guest-memory, device-state, checkpoint, migration, atomic/quiesced snapshot, or rollback primitive.

`Vcpu::run_once` retries an interrupted host syscall, performs one completed `KVM_RUN`, reads only tested x86 `kvm_run` prefixes, and returns a typed `VcpuExit`. HLT, port I/O, legacy shutdown, KVM unknown, exception, fail-entry, internal-error, and system-event exits are classified explicitly; other unsupported reasons retain their exact raw reason.

For `KVM_EXIT_IO`, `Vcpu::port_io_exit` validates direction, `data_offset`, checked `size * count`, the complete mapped range, and owned OUT copying. `Vcpu::write_port_io_input` independently validates IN direction and exact response length before writing owned bytes to the pending KVM data range. No pointer into `kvm_run` leaves the vCPU layer.

Purpose-built KVM-unknown, exception, fail-entry, internal-error, and system-event decoders validate the current reason before inspecting their union member, copy required fields into owned Rust state, bound every variable-length count before slicing, and keep higher-level dispatch free of raw shared-memory pointers.

## Bounded execution loop

`execution::run_vcpu_until_stopped` is the single reusable run-loop boundary for the current one-vCPU model. Before each `KVM_RUN` it checks an explicit completed-exit budget. A successful `KVM_RUN` consumes exactly one budget unit; host-side failures that do not produce a completed VM exit consume none.

Each completed exit is recorded exactly once in an ordered raw reason trace before dispatch. Serviceable I/O is recorded as an owned `PortIoExit` and execution continues while budget remains. A terminal HLT or legacy shutdown returns `VmExecutionResult`, which contains the terminal `VmExitReport`, every serviced typed I/O exit, the exact completed-exit count, and the complete ordered raw reason trace.

A zero budget fails before any guest run. Budget exhaustion is structured failure, not guest termination. If the final permitted exit was serviceable I/O, userspace may have prepared the service response but the VMM does not claim KVM completed the pending operation without another permitted `KVM_RUN`.

The HLT and CPUID fixtures use budget 1. The real-mode debug-port fixtures use budget 2. The x86-64 long-mode fixture uses budget 5 and succeeds only with the exact sequence of four serviced I/O exits followed by terminal HLT; extra exits consume the budget and prevent milestone success.

## VM-exit dispatch

`vmexit::dispatch_vcpu_exit` is the single policy boundary for one completed vCPU exit. HLT and legacy shutdown become `VmExitReport`; I/O is serviced through `PortIoBus`; KVM-unknown, exception, fail-entry, internal-error, and system-event exits become their existing structured diagnostics; other unsupported raw reasons remain generic unhandled errors.

The dispatcher deliberately does not snapshot registers for an in-flight KVM I/O exit because KVM defines the operation as pending until userspace re-enters `KVM_RUN`. Register state used as a completed-operation diagnostic is therefore taken on the later terminal exit.

The deterministic real-mode output fixture reaches HLT at RIP `0x1005`; the input fixture reaches `0x1006`; the CPUID fixture reaches `0x101c`. The x86-64 long-mode fixture emits `LM64` across four I/O exits and reaches HLT at RIP `0x10024`.

## Port-I/O bus and debug device

`PortIoBus` remains intentionally minimal. It contains only the exact debug-port device at port `0xe9`; it is not a dynamic device registry or port-range resolver. The device accepts only byte-wide, single-count accesses. OUT appends one copied byte to the output buffer; IN returns one configured owned byte. Unknown ports, wide/multi-count operations, payload mismatches, and response-length mismatches are explicit errors.

The long-mode fixture reuses this exact existing path; no new device model was introduced for the milestone.

## Ownership and lifetime

`KvmBackend` owns `/dev/kvm`, validated capability and CPUID/MSR discovery snapshots, and the configured guest CPU policy. `Vm` owns the VM descriptor, guest policy, optional internal-error capability observation, and registered guest RAM. `Vcpu` owns the vCPU descriptor and `KvmRunMapping`. CPU/MSR snapshots, diagnostics, `PortIoExit`, `VmExecutionResult`, and fixture result types own copied Rust data rather than pointers into KVM shared memory or guest RAM.

`LongModeBootLayout` owns only validated guest-physical layout scalars; it contains no host pointer, mapping borrow, vCPU descriptor, or raw KVM state. Page-table installation mutates guest RAM only through checked `GuestMemory` writes. `LongModeGuestResult` owns its copied I/O exits, proof bytes, and terminal report.

Rust ownership is used for normal cleanup; explicit KVM slot removal protects the guest-RAM lifetime boundary when independent vCPU descriptors exist.

## Error boundary

Errors remain categorized as host environment, KVM capability, configuration, guest memory, guest image, VM exit, and port I/O errors. Pure guest-MSR policy/value/snapshot validation keeps its dedicated typed errors.

Long-mode layout validation is a pure configuration boundary represented by `LongModeConfigurationError`; invalid RAM base/size, entry mapping, page-table overlap, stack mapping, or stack/page-table overlap is rejected before page-table installation or KVM long-mode state configuration. Page-table writes still use the existing `GuestMemory` error boundary. `Vcpu::initialize_long_mode` uses the existing named `KVM_GET_SREGS`, `KVM_SET_SREGS`, and `KVM_SET_REGS` vCPU-operation errors. Runtime proof failure remains an execution/VM-exit failure rather than being converted into successful milestone completion.

Future MMIO, interrupt, whole-VM/device-snapshot, and stronger invariant categories will be added only when those responsibilities exist.

## Deliberate non-abstractions

There is no generic hypervisor backend trait yet. KVM is the only implementation, and an abstraction would not have a second consumer.

There is no configurable or migration-stable CPU model yet. Current CPUID and immutable-MSR model-candidate boundaries remain host-derived analysis contracts rather than cross-host migration guarantees.

The implemented state lifecycle is deliberately vCPU-CPU-state scoped and non-transactional across multi-component restore. There is no automatic mismatch repair, rollback, multi-vCPU restore orchestration, guest-memory/device snapshot, checkpoint decoder, or migration protocol.

The long-mode mapping is deliberately **not** a generic virtual-memory subsystem. The milestone owns one fixed 2 MiB identity map and three fixed page-table pages solely to establish deterministic long-mode execution. There is no allocator for page-table pages, no arbitrary VA→GPA mapping API, no page-permission policy surface, and no guest-controlled page-table construction path.

Typed KVM-unknown, exception, fail-entry, internal-error, and system-event diagnostics remain diagnostics. They do not imply retry, recovery, exception injection, instruction emulation, placement, or lifecycle policy.

There is no multi-region memory map yet. `GuestMemoryRegion::overlaps` exists to make range semantics explicit and tested, but the VM intentionally supports only slot 0.

The port bus is not a trait-object registry yet. One exact bidirectional device is enough to prove checked OUT/IN behavior and the long-mode proof without introducing registration/range-resolution machinery prematurely.

The execution loop is not a scheduler. It owns no vCPU, thread, timer, or interrupt state; it only bounds repeated execution of one already-created vCPU.

## Next architectural milestone

No architectural milestone is selected in this document. `ROADMAP.md` is the authoritative live source for future selection. After the x86-64 long-mode milestone is integrated and exact post-merge `main` CI is green, development stops until a later milestone is explicitly chosen. ELF, Linux boot, MMIO, interrupts/APIC, virtio, SMP, snapshots, migration, and resumable execution are not automatically authorized follow-ons.

## Internal-error emulation-failure metadata

`VcpuInternalError` exposes read-only interpretation for the stable x86 `KVM_INTERNAL_ERROR_EMULATION` metadata already copied into its owned optional-data words. These accessors are pure reads over already-owned diagnostic state, form no additional `kvm_run` view, perform no ioctl, and introduce no emulation recovery or execution policy.

## KVM exception diagnostics

`KVM_EXIT_EXCEPTION` raw reason `1` is a distinct typed `VcpuExit::Exception` path. `Vcpu::exception_exit()` validates the reason before reading the fixed x86 union member, copies exception vector/error code into owned `VcpuException`, and dispatch returns the existing structured diagnostic without a secondary register ioctl. Exception metadata remains opaque and grants no injection, reinjection, emulation, retry, recovery, or lifecycle authority.
