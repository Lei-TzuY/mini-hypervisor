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
              ├─ explicit MsrIndex[] → bounded KVM_GET_MSRS → VcpuMsrValues
              ├─ GuestMsrValueSet → bounded KVM_SET_MSRS
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

`GuestCpuPolicy::compare` is a pure compatibility/diff primitive over two already-configured guest CPUID policies. The left side is the reference and the right side is the observed policy. The project-level identity key is `CpuidPolicyKey(function, index)`, so list order is not part of policy identity: reordered entries with identical keys, flags, and architectural registers compare as an exact configured contract match. A function or index change creates a reference-missing plus observed-extra entry rather than a same-key mismatch.

For entries with the same policy key, `CpuidPolicyEntryMismatch` retains both complete `CpuidEntry` values and reports exactly which contract fields differ. `CpuidPolicyField` uses the canonical order `Flags`, `Eax`, `Ebx`, `Ecx`, `Edx`; function and index are identity rather than mismatch fields. Missing entries and same-key mismatches follow reference policy order, while extras follow observed policy order. This comparison is intentionally about the stored configured policy records; it does not reinterpret raw CPUID execution/index-significance semantics.

Every `GuestCpuPolicyComparison` owns clones of both complete policies in addition to its findings. The comparison is therefore independent of backend/vCPU lifetimes and preserves the exact configured inputs used for the analysis. Focused regressions lock identity-key behavior, identical and reordered policies, missing/extra entries, all-field and single-field mismatches, directional finding order, complete policy retention, and empty-policy behavior.

An exact guest CPUID policy comparison remains weaker than migration compatibility. It states only that the configured `(function,index) → flags/EAX/EBX/ECX/EDX` mappings agree under this project contract. It does not establish matching MSR capabilities, kernel behavior, device state, memory layout, or any other migration requirement.

`KvmBackend` retains both the discovered `HostCpuid` and derived `GuestCpuPolicy`. A `Vm` receives only the guest policy. `Vm::create_vcpu` performs `KVM_CREATE_VCPU`, serializes the policy into a fresh zero-padded KVM buffer, applies it with `KVM_SET_CPUID2`, then immediately reads the created vCPU contract back through a second fixed 256-entry buffer using `KVM_GET_CPUID2`. The read-back `nent` is independently validated before any slice is formed; entries are converted into owned `CpuidEntry` values and must exactly match the submitted policy in count, order, function, index, flags, and architectural registers. Only after that verification does the method construct/return `Vcpu`. SET, GET, count-validation, or comparison failures close the fresh descriptor through `OwnedFd` drop.

The deterministic CPUID fixture proves the configured contract from inside the guest rather than trusting host-side policy construction and read-back alone. In real mode it executes `CPUID(1)`, stores ECX at guest physical `0x2000`, executes `CPUID(0x40000001)`, stores EAX at `0x2004`, then halts. After the terminal exit, host code reads the checked eight-byte result range and exposes the observations through `CpuidGuestResult`. The integration regression requires x2APIC, TSC-deadline, and PV-unhalt to remain clear in guest-observed state.

### Host MSR capability snapshots

Both MSR-index discovery paths are system ioctls over the same variable-length `struct kvm_msr_list` ABI: a 4-byte `nmsrs` header followed by `u32` indices. Pure tests lock the header size, trailing-index offset, and the exact request values for `KVM_GET_MSR_INDEX_LIST` and `KVM_GET_MSR_FEATURE_INDEX_LIST`.

`KVM_GET_MSR_INDEX_LIST` describes the general MSR indices KVM exposes through its MSR access interface. Discovery is deliberately two-stage. First, `KvmBackend` submits a zero-capacity `KvmMsrList<0>` probe. KVM writes the required count back into `nmsrs` before returning `E2BIG` when the supplied capacity is insufficient, so `E2BIG` is accepted only for this probe. The reported count must be in `1..=1024`, where 1024 is this project's defensive bound rather than a claim about a Linux ABI maximum. The backend then issues a second query with a fixed 1024-entry `repr(C)` buffer and independently validates the final returned count before any Rust slice is formed.

`KVM_GET_MSR_FEATURE_INDEX_LIST` is a separate capability-discovery contract guarded by `KVM_CAP_GET_MSR_FEATURES`. It reports the feature-MSR indices that may be supplied to the system-level `KVM_GET_MSRS` interface for host/KVM feature probing. It uses the same zero-capacity probe and fixed 1024-entry second query, but its validated count is `0..=1024`: an empty feature list is a valid capability snapshot and is not treated as malformed. The second returned count is independently checked before slicing just like the general list.

Validated raw indices are copied into distinct owned typed snapshots. Both lists reuse `MsrIndex`, and both normalize repeated kernel indices by keeping the first occurrence while preserving the kernel's reported order for retained entries. `HostMsrIndexList` represents the general MSR access set; `HostMsrFeatureIndexList` represents the system feature-probing set. Keeping these types separate prevents the two kernel contracts from becoming interchangeable merely because their wire representation is identical. Pure tests lock order preservation, duplicate normalization, typed reuse, and the valid empty feature-list case. Raw variable-length KVM buffers do not escape the KVM module.

The backend then reads the normalized feature indices through the system form of `KVM_GET_MSRS`. `KvmMsrs<N>` models the exact variable-length UAPI as an 8-byte `nmsrs`/padding header followed by 16-byte `KvmMsrEntry` values. Its constructor zeroes the header padding and every entry's reserved/data fields. Before the unsafe ioctl boundary, the wrapper independently requires the requested `nmsrs` to fit the actual backing array, so a malformed userspace header cannot authorize a kernel copy beyond the fixed Rust object.

`KVM_GET_MSRS` returns the number of entries successfully processed rather than rewriting `nmsrs` as a completion count. The backend therefore requires the returned count to equal the complete requested feature-index count. A partial result is rejected as malformed host-discovery state and identifies the first unread feature index when one exists; a returned count greater than requested is also rejected. Before any data becomes typed state, each returned entry index must still equal the requested index at the same position. Only a complete index-stable response becomes owned `MsrFeatureValue` entries inside `HostMsrFeatureValues`. An empty feature-index snapshot produces an empty value snapshot without issuing the value ioctl.

Every `MsrFeatureValue` now carries an `MsrFeatureStability` assigned inside its crate-private constructor, so callers cannot attach inconsistent stability metadata. Linux KVM treats feature MSRs as immutable once the vCPU model is defined except for `MSR_IA32_UCODE_REV`, which tracks the currently loaded microcode patch. The exact architectural index `0x8b` is therefore classified `HostMutable`; every other value returned through the current KVM feature-MSR contract is classified `ModelImmutable`. `HostMsrFeatureValues::model_immutable_values` and `host_mutable_values` expose order-preserving, mutually exclusive views over the same owned snapshot, and focused tests lock the `0x8b` constant plus partition behavior.

`ModelImmutable` is deliberately a narrow KVM-model statement, not a migration guarantee. It means KVM treats that feature value as immutable after the vCPU model is defined; it does not claim that two hosts, kernel versions, CPU revisions, or VMM configurations will expose the same value. `HostMutable` makes the microcode-revision exception explicit so it cannot be silently consumed as though it belonged to an immutable model-capability set.

`HostMsrFeatureValues::model_candidate` is the single public materialization path for `HostMsrModelCandidate`. The candidate owns a clone of the complete source observation as provenance and separately owns only the source entries classified `ModelImmutable`, preserving their original order and values. Candidate fields and its internal constructor are private, so callers cannot construct a candidate that injects `HostMutable` entries or mismatches the immutable subset with unrelated provenance. All-mutable and empty observations therefore produce valid empty candidate value sets while still retaining the complete source observation. Materialization is pure host-side Rust code and issues no additional KVM operation.

`HostMsrModelCandidate::compare` is a pure host-side compatibility/diff primitive over two immutable candidate value sets. The left side is the reference and the right side is the observed candidate. Matching is keyed by `MsrIndex`, so two candidates with the same immutable index/value mapping are an exact model match even when the source order differs. Reference entries whose indices are absent from the observed candidate become `missing_from_observed`; observed entries absent from the reference become `extra_in_observed`; equal indices with different data become `MsrModelValueMismatch` records carrying the index plus both values. Missing entries and value mismatches follow reference candidate order, while extras follow observed candidate order.

Every `HostMsrModelComparison` owns clones of both complete candidates in addition to its findings. Because each candidate already owns its complete `HostMsrFeatureValues` source observation, comparison results retain both provenance chains without consulting those source observations to decide compatibility. In particular, different `HostMutable` source values such as `MSR_IA32_UCODE_REV` cannot create a model difference because they were excluded before candidate construction. Focused regressions lock identical, reordered, missing, extra, value-mismatch, mixed, mutable-only-drift, provenance, and empty cases.

An exact MSR model-candidate match is deliberately weaker than migration compatibility. It states only that the two immutable candidate index/value mappings agree under this contract. It does not validate CPUID policy, kernel behavior, device state, memory layout, guest MSR lifecycle, or any other migration requirement.

The general-index, feature-index, and feature-value snapshots remain owned only by `KvmBackend` and are exposed read-only. Model candidates and comparisons are materialized on demand from owned safe Rust state rather than duplicated as backend state. None is copied into `Vm` or `Vcpu`, and no guest MSR policy is derived from the feature snapshot. The system-level feature query remains host capability discovery; caller-selected guest MSR access policy and vCPU architectural-state readback are separate boundaries described below.

### Guest MSR access policy

`GuestMsrAccessPolicy::from_host` is a pure configuration boundary over a validated general `HostMsrIndexList` and an explicit caller-selected `MsrIndex` slice. It issues no ioctl, does not inspect a vCPU, and does not attach MSR values. An empty requested slice is valid and produces an empty owned policy.

Every requested index must appear in the general `KVM_GET_MSR_INDEX_LIST` snapshot. Unsupported indices are rejected with their caller position before any policy is returned. Caller order is preserved exactly for successful entries. Unlike host discovery, which normalizes duplicate kernel reports, guest policy construction treats a duplicate caller request as a configuration error and reports both the first and duplicate positions. The constructor therefore never silently changes authorization intent.

A successful policy contains owned `GuestMsrAccess` entries. Each entry currently carries `MsrAccessAuthority::ReadWrite`, meaning the VMM authorizes future read and write **attempts** for that index. This authority is narrower than a value-validity guarantee: Linux KVM may still reject a particular future `KVM_SET_MSRS` value because of reserved bits, architectural constraints, or other per-MSR semantics. The policy contains no `u64` MSR data and does not claim that arbitrary values are writable.

Policy construction is all-or-nothing. Unsupported or duplicate input returns `GuestMsrPolicyError` and no `GuestMsrAccessPolicy`; any locally accumulated entries are dropped. The successful policy owns its entries independently of the host capability snapshot and caller slice. It is not derived from `HostMsrFeatureValues`, `HostMsrModelCandidate`, `CpuModelCandidate`, or `VcpuMsrValues`, and membership is not a migration-safety statement.

### Guest MSR value set

`GuestMsrValueSet::from_policy` is a pure state-materialization boundary over an already validated `GuestMsrAccessPolicy` and an explicit caller sequence of `(MsrIndex, u64)` values. It issues no ioctl and does not inspect or mutate a vCPU. Empty input is valid.

A value set is intentionally allowed to be a strict subset of the policy. Policy breadth describes which MSR indices the VMM authorizes for future read/write attempts; one concrete value set may represent only the state fragment needed by a caller. Every supplied index must nevertheless match a `ReadWrite` policy entry. An unauthorized index is rejected with its caller position.

Caller order is preserved exactly. Duplicate value indices are rejected rather than normalized and report both the first and duplicate positions. Validation is all-or-nothing: no `GuestMsrValueSet` is returned after an unauthorized or duplicate input, and any locally accumulated values are dropped.

Successful results own only typed `GuestMsrValue` records containing `MsrIndex` plus `u64`. They keep no borrow into the policy or caller slice and carry no `MsrFeatureStability`, host feature observation, model-candidate provenance, or vCPU descriptor state. Authorization and value-set validation do not prove that KVM will accept a particular write value; per-MSR architectural constraints remain kernel semantics. The value set is not a migration-safety statement and causes no mutation until it is explicitly passed to the vCPU write boundary.

### vCPU MSR writes

`Vcpu::set_msrs` is the only current guest-MSR write entry point. It accepts an already-validated `GuestMsrValueSet`, not an arbitrary tuple slice, so callers cannot bypass the `HostMsrIndexList → GuestMsrAccessPolicy → GuestMsrValueSet` authorization path at the write call itself. The write primitive does not derive state from `HostMsrFeatureValues`, `HostMsrModelCandidate`, `CpuModelCandidate`, or `VcpuMsrValues`.

An empty value set returns success without issuing `KVM_SET_MSRS`. A non-empty set is serialized into a fresh zero-initialized `KvmMsrs<1024>` buffer in exact value-set order. Only the active entries' `index` and `data` fields are filled; the header `pad`, each entry `reserved` field, and unused entries remain zero. The vCPU layer rejects more than 1024 values before the ioctl, and `sys::set_msrs` independently validates the encoded `nmsrs` against its actual const-generic backing capacity immediately before the unsafe call.

KVM's successful return is the number of entries processed, not an atomic transaction status. Exact completion is success. A short return becomes `HostEnvironmentError::VcpuMsrPartialWrite` containing the vCPU id, requested count, processed count, and the first unwritten MSR index. The already-processed prefix may have mutated architectural state, so the VMM does not retry, roll back, or describe the failure as atomic. A returned count greater than requested becomes `VcpuMsrInvalidWriteCompletion` and is rejected as malformed completion metadata.

Policy authorization still does not guarantee per-value acceptance. KVM may reject a write because of reserved bits or other architectural constraints even when the index is authorized. This primitive performs one bounded write attempt only; it does not automatically read back state, sequence multiple state classes, orchestrate restore, or claim migration safety.

### vCPU MSR readback

`Vcpu::msrs` reads architectural MSR state only from an already-created vCPU descriptor and only for the exact `MsrIndex` slice supplied by its caller. It does not consult `HostMsrFeatureValues`, `HostMsrModelCandidate`, or any implicit model candidate to decide what to read. The KVM-aware regression uses indices from `HostMsrIndexList` only as explicit caller-selected supported inputs; that test choice does not create an API dependency from vCPU state to the host feature snapshot.

An empty caller request returns an empty `VcpuMsrValues` immediately and does not issue `KVM_GET_MSRS`. Non-empty requests use a fixed `KvmMsrs<1024>` buffer. The readback layer rejects a caller count above 1024 before the ioctl, while `sys::get_msrs` independently rechecks the encoded `nmsrs` against its actual const-generic backing capacity immediately before the unsafe call.

Request construction copies every caller index into the KVM entries in the same order and deliberately does not normalize duplicates. KVM's successful return value is treated as an untrusted completion count: it must equal the requested count exactly. A partial completion reports the first unread requested index; an impossible over-completion is rejected. The response entry slice must also contain the complete requested prefix, and every returned entry index must equal the caller's index at the same position before any typed values are published.

Only a fully completed, position-stable response becomes owned `VcpuMsrValue` entries inside `VcpuMsrValues`. These types contain only `MsrIndex` plus the architectural `u64` value and intentionally carry no `MsrFeatureStability`, because system feature-MSR stability classification is not guest vCPU-state metadata. The result owns its values and contains no pointer or borrow into the vCPU descriptor or KVM request buffer.

Readback and write are deliberately separate primitives. `Vcpu::msrs` does not consult `GuestMsrAccessPolicy` automatically, while `Vcpu::set_msrs` accepts only an already policy-validated `GuestMsrValueSet`. Neither operation automatically adds vCPU state to `CpuModelCandidate` or `CpuModelComparison`, and neither makes a migration-stability claim.

### CPU model candidate composition

`CpuModelCandidate` is a pure owned composition boundary above the independent CPUID and immutable-MSR contracts. Construction takes references to one already-configured `GuestCpuPolicy` and one already-derived `HostMsrModelCandidate`, then clones those exact typed values. It does not query KVM, rebuild or remask CPUID entries, reclassify MSR values, or derive a guest MSR policy.

The composed value exposes both components read-only and owns them independently of the source variables. Because the embedded `HostMsrModelCandidate` already owns its complete `HostMsrFeatureValues` source observation, composition preserves the full MSR provenance chain, including excluded `HostMutable` observations such as `MSR_IA32_UCODE_REV`. Empty CPUID and empty immutable-MSR components remain valid composition inputs. Focused regressions lock exact component round-trip, ownership after the source variables are dropped, provenance retention, empty components, and clone stability.

`CpuModelCandidate::compare` is a pure report-composition boundary. It delegates the configured CPUID portion directly to `GuestCpuPolicy::compare` and the immutable-MSR portion directly to `HostMsrModelCandidate::compare`, then stores those two already-defined reports inside an owned `CpuModelComparison`. The composition layer performs no independent key/index matching and does not reinterpret either component's diagnostics.

`CpuModelComparison` exposes the retained `GuestCpuPolicyComparison` and `HostMsrModelComparison` read-only. Their original reference/observed direction, directional findings, full configured policies, immutable-MSR candidate values, and MSR source-observation provenance therefore remain intact. Focused regressions require the composed component reports to be exactly equal to direct component comparisons for exact, CPUID-only-drift, MSR-only-drift, dual-drift, empty, and ownership cases.

Neither `CpuModelCandidate` nor `CpuModelComparison` exposes a combined migration-safe or stronger compatibility verdict. They are typed analysis units that keep the configured CPUID and immutable host-MSR contracts together without weakening either component's existing semantics.

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

`Vcpu::msrs` and `Vcpu::set_msrs` are synchronous state operations over the same owned vCPU descriptor used by the execution path. The current VMM has no scheduler or concurrent vCPU runner; callers invoke these primitives directly rather than racing an independently executing vCPU thread. Readback returns owned typed values; writes consume only an owned, policy-validated value set and retain no reference to it after the ioctl returns.

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

`KvmBackend` owns `/dev/kvm`, validated host capabilities, the typed `HostCpuid`, `HostMsrIndexList`, `HostMsrFeatureIndexList`, and stability-annotated `HostMsrFeatureValues` discovery snapshots, and the derived `GuestCpuPolicy`. `GuestMsrAccessPolicy` is an on-demand owned derivative of the validated general MSR capability set plus explicit caller authorization; it keeps no borrow into `KvmBackend` or the caller slice. `GuestMsrValueSet` is another on-demand owned derivative of an access policy plus explicit caller state; it owns only typed MSR index/value pairs and keeps no borrow into the policy or caller slice. `GuestCpuPolicyComparison` is an on-demand owned derivative that clones both complete configured policies plus all directional findings; it holds no borrow into backend or VM state. `HostMsrModelCandidate` is an on-demand owned derivative: it clones the complete feature-value observation as provenance and separately owns only its immutable subset. `HostMsrModelComparison` is also owned: it clones both complete candidates and owns its missing/extra/value-mismatch findings, so comparison data carries no borrow into backend state. `CpuModelCandidate` is another owned derivative that clones one complete configured `GuestCpuPolicy` plus one complete `HostMsrModelCandidate`; the latter continues to own its complete source-observation provenance. `CpuModelComparison` owns the resulting `GuestCpuPolicyComparison` and `HostMsrModelComparison`, so the composed report has no borrow into either source candidate and retains both component provenance chains. `Vm` owns the VM descriptor, a clone of only the guest CPUID policy, and its optional registered guest RAM. CPUID read-back buffers and decoded comparison entries are temporary data inside vCPU construction; neither is retained after exact verification. `Vcpu` owns the vCPU descriptor and `KvmRunMapping`; each successful `Vcpu::msrs` call returns a separate owned `VcpuMsrValues` snapshot with no descriptor or UAPI-buffer borrow, while `Vcpu::set_msrs` serializes a borrowed `GuestMsrValueSet` into a temporary bounded UAPI buffer and retains nothing from the caller after return. `PortIoBus` owns its optional debug device, configured input byte, and accepted output bytes. `VmExecutionResult` and `CpuidGuestResult` own only copied safe Rust data and reports; neither contains a pointer or borrow into KVM shared memory or guest RAM.

Rust ownership is used for normal cleanup; explicit KVM slot removal protects the guest-RAM lifetime boundary when independent vCPU descriptors exist.

## Error boundary

Errors are categorized as:

- `HostEnvironment`: host file/device/I/O failures, malformed host KVM variable-length responses including CPUID, general MSR-index, MSR-feature-index, system MSR-feature-value completion/index semantics, vCPU MSR read request/response validation, vCPU MSR write request/completion validation including structured non-transactional partial writes, named VM/vCPU ioctls including CPUID query/application/read-back plus `KVM_GET_MSRS`/`KVM_SET_MSRS`, and CPUID read-back policy mismatches;
- `KvmCapability`: incompatible API version, absent required extension, or invalid kernel-reported mapping size;
- `Configuration`: unsupported VMM configuration or current real-mode entry limits;
- `GuestMemory`: invalid guest ranges, reserved-range overlap, mapping failures, bounds violations, or KVM RAM-registration failures;
- `GuestImage`: malformed or overflowing flat-image descriptions;
- `VmExit`: unsupported exits, malformed KVM I/O metadata/ranges, invalid IN response direction/length, execution-budget exhaustion, or deterministic fixture sequence failures;
- `PortIo`: unknown ports or unsupported/malformed device accesses.

Pure guest-MSR policy construction has its own `GuestMsrPolicyError`, while pure value-set materialization has `GuestMsrValueSetError`. Unsupported/duplicate caller authorization and unauthorized/duplicate caller value state are configuration/state validation failures and do not originate from host I/O or a vCPU operation. `VcpuMsrPartialWrite` is intentionally different: it reports that a kernel write attempt stopped after a prefix that may already have changed vCPU state. Future MMIO, interrupt, snapshot, and stronger invariant categories will be added only when those responsibilities exist.

## Deliberate non-abstractions

There is no generic hypervisor backend trait yet. KVM is the only implementation, and an abstraction would not have a second consumer. The KVM-specific plumbing is nevertheless isolated so a later raw-VMX research backend would not require leaking ioctls into VM policy.

There is no configurable or migration-stable CPU model yet. The current boundary distinguishes discovered host support from a derived guest CPUID policy, but there is still exactly one built-in CPUID policy: host/KVM-supported entries with conservative masking for the absent LAPIC model. The vCPU creation path requires KVM to report back exactly that submitted typed CPUID policy, and the existing CPUID fixture independently verifies selected architectural bits from guest-observed state. `GuestCpuPolicyComparison`, `HostMsrIndexList`, `HostMsrFeatureIndexList`, stability-annotated `HostMsrFeatureValues`, `HostMsrModelCandidate`, `HostMsrModelComparison`, `CpuModelCandidate`, and `CpuModelComparison` remain analysis state; composed comparison does not convert component-level exact matches into a named cross-host CPU model or migration guarantee. `VcpuMsrValues` is a caller-directed observation of one vCPU's architectural MSR values. `GuestMsrAccessPolicy` defines explicit caller-selected MSR access authority bounded by the general host-supported set, `GuestMsrValueSet` materializes an owned policy-validated subset of caller state, and `Vcpu::set_msrs` now exposes one bounded non-transactional write attempt. None is migration-stable, and value-set membership does not guarantee per-value KVM acceptance. There is still no policy-bound capture primitive, multi-step restore orchestration, automatic read-after-write verification, or rollback.

There is also no multi-region memory map yet. `GuestMemoryRegion::overlaps` exists to make range semantics explicit and tested, but the VM intentionally supports only slot 0 in this milestone.

The port bus is not a trait-object registry yet. One exact bidirectional device is enough to prove OUT copying and IN response write-back without introducing registration/range-resolution machinery prematurely.

The execution loop is not a scheduler. It owns no vCPU, thread, timer, or interrupt state; it only bounds repeated execution of one already-created vCPU.

## Next architectural milestone

The next bounded slice should add a policy-bound vCPU MSR capture primitive that takes an existing `GuestMsrAccessPolicy`, reads exactly those authorized indices through the existing bounded `Vcpu::msrs` path in policy order, and materializes the fully completed readback into an owned `GuestMsrValueSet`. An empty policy should return an empty value set without issuing `KVM_GET_MSRS`; non-empty capture must inherit the existing 1024-entry bound and all-or-nothing readback validation rather than exposing a successful prefix. Focused tests should lock policy-order extraction, empty behavior, exact value transfer, and failure propagation without adding new raw ioctls. The result must still be only a captured state fragment: this slice must not call `KVM_SET_MSRS`, automatically restore another vCPU, retry partial writes, combine register/CPUID/device state, claim migration safety, or add long-mode boot, interrupts, MMIO, SMP, or device expansion in the same slice.