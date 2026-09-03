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
       └─ vCPU creation
              ├─ KVM_CREATE_VCPU
              ├─ GuestCpuPolicy → KVM_SET_CPUID2
              ├─ bounded KVM_GET_CPUID2 → typed exact verification
              ├─ propagate optional internal-error-data support
              ↓
             Vcpu
              ├─ explicit real-mode register setup
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
              ├─ checked KVM_EXIT_FAIL_ENTRY payload extraction
              ├─ capability-gated KVM_EXIT_INTERNAL_ERROR payload extraction
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
                   ├─ FAIL_ENTRY → structured entry-failure diagnostic
                   ├─ INTERNAL_ERROR → structured capability-aware diagnostic
                   ├─ SYSTEM_EVENT → structured unsupported diagnostic
                   └─ unknown raw reason → VmExitError::Unhandled
```

The raw ioctl UAPI details live in `src/kvm/sys.rs`; the tested `KVM_EXIT_FAIL_ENTRY`, capability-gated `KVM_EXIT_INTERNAL_ERROR`, and `KVM_EXIT_SYSTEM_EVENT` `kvm_run` payload views are isolated in `src/vcpu/fail_entry.rs`, `src/vcpu/internal_error.rs`, and `src/vcpu/system_event.rs`. Higher layers call typed Rust methods and do not issue raw `ioctl` operations or inspect raw shared-memory payload layouts directly.

## x86 host and CPU capability contract

The backend requires KVM API version 12 plus `KVM_CAP_USER_MEMORY`, `KVM_CAP_SET_TSS_ADDR`, `KVM_CAP_EXT_CPUID`, `KVM_CAP_SET_IDENTITY_MAP_ADDR`, and `KVM_CAP_GET_MSR_FEATURES`.

Separately from that required set, `KvmBackend` queries `KVM_CAP_INTERNAL_ERROR_DATA` (capability ID 40) through the same `KVM_CHECK_EXTENSION` boundary and stores the raw returned value in the existing owned `HostCapabilities.extensions` snapshot. A missing observation in manually constructed `HostCapabilities` or a recorded value of `0` does not invalidate an otherwise valid host; `internal_error_data_capability()` exposes the recorded observation when present and `supports_internal_error_data()` is true exactly when its raw value is greater than zero. `KvmBackend::create_vm` propagates only that support boolean into `Vm`, and each created `Vcpu` inherits the same fact. A non-positive observation keeps internal-error decoding on the always-available base `suberror` view. A positive observation authorizes the vCPU decoder to form the fixed full x86 internal-error payload view and validate/copy declared optional data; it still does not create emulation recovery, retry, replacement execution, or lifecycle policy.

After fixed host capability validation, `KvmBackend` performs `KVM_GET_SUPPORTED_CPUID` through a fixed 256-entry `repr(C)` `KvmCpuid2<N>` buffer. The header remains the exact 8-byte KVM ABI header and each trailing `KvmCpuidEntry2` is the exact 40-byte x86 entry. Pure tests lock header size, entry size, entry offset, and the supported/set/get CPUID ioctl request numbers.

The kernel-returned `nent` is not trusted as a Rust slice length. It must be non-zero and no greater than the fixed capacity before the entry prefix is converted into owned `CpuidEntry` values. Reserved KVM padding is intentionally absent from the typed representation, and conversion back to KVM UAPI always writes zero padding.

Host discovery and guest policy are separate states. `HostCpuid` preserves the validated KVM-supported function/index/flags/register values exactly; it contains no guest feature masking. `GuestCpuPolicy::from_host` clones that snapshot and applies the current no-LAPIC policy as a pure transformation. Pure tests require that construction not mutate the host snapshot, that unrelated metadata/registers are preserved, and that every matching indexed entry is handled consistently.

The current interrupt model has no in-kernel LAPIC or IRQ chip. Linux KVM documents x2APIC, TSC-deadline, and `KVM_FEATURE_PV_UNHALT` as depending on that interrupt model, so `GuestCpuPolicy` removes those three bits. No additional CPU feature is synthesized. This remains a host-derived runtime contract, not a migration-stable named CPU model.

`GuestCpuPolicy::compare` is a pure compatibility/diff primitive over two already-configured guest CPUID policies. The left side is the reference and the right side is the observed policy. The project-level identity key is `CpuidPolicyKey(function, index)`, so list order is not part of policy identity: reordered entries with identical keys, flags, and architectural registers compare as an exact configured contract match. A function or index change creates a reference-missing plus observed-extra entry rather than a same-key mismatch.

For entries with the same policy key, `CpuidPolicyEntryMismatch` retains both complete `CpuidEntry` values and reports exactly which contract fields differ. `CpuidPolicyField` uses the canonical order `Flags`, `Eax`, `Ebx`, `Ecx`, `Edx`; function and index are identity rather than mismatch fields. Missing entries and same-key mismatches follow reference policy order, while extras follow observed policy order. This comparison is intentionally about the stored configured policy records; it does not reinterpret raw CPUID execution/index-significance semantics.

Every `GuestCpuPolicyComparison` owns clones of both complete policies in addition to its findings. The comparison is therefore independent of backend/vCPU lifetimes and preserves the exact configured inputs used for the analysis. Focused regressions lock identity-key behavior, identical and reordered policies, missing/extra entries, all-field and single-field mismatches, directional finding order, complete policy retention, and empty-policy behavior.

An exact guest CPUID policy comparison remains weaker than migration compatibility. It states only that the configured `(function,index) → flags/EAX/EBX/ECX/EDX` mappings agree under this project contract. It does not establish matching MSR capabilities, kernel behavior, device state, memory layout, or any other migration requirement.

`KvmBackend` retains both the discovered `HostCpuid` and derived `GuestCpuPolicy`. A `Vm` receives only the guest policy. `Vm::create_vcpu` performs `KVM_CREATE_VCPU`, serializes the policy into a fresh zero-padded KVM buffer, applies it with `KVM_SET_CPUID2`, then immediately reads the created vCPU contract back through a second fixed 256-entry buffer using `KVM_GET_CPUID2`. The read-back `nent` is independently validated before any slice is formed; entries are converted into owned `CpuidEntry` values and must exactly match the submitted policy in count, order, function, index, flags, EAX, EBX, ECX, and EDX. Only after that verification does the method construct/return `Vcpu`. SET, GET, count-validation, or comparison failures close the fresh descriptor through `OwnedFd` drop.

The deterministic CPUID fixture proves the configured contract from inside the guest rather than trusting host-side policy construction and read-back alone. In real mode it executes `CPUID(1)`, stores ECX at guest physical `0x2000`, executes `CPUID(0x40000001)`, stores EAX at `0x2004`, then halts. After the terminal exit, host code reads the checked eight-byte result range and exposes the observations through `CpuidGuestResult`. The integration regression requires x2APIC, TSC-deadline, and PV-unhalt to remain clear in guest-observed state.

### Host MSR capability snapshots

Both MSR-index discovery paths are system ioctls over the same variable-length `struct kvm_msr_list` ABI: a 4-byte `nmsrs` header followed by `u32` indices. Pure tests lock the header size, trailing-index offset, and the exact request values for `KVM_GET_MSR_INDEX_LIST` and `KVM_GET_MSR_FEATURE_INDEX_LIST`.

`KVM_GET_MSR_INDEX_LIST` describes the general MSR indices KVM exposes through its MSR access interface. Discovery is deliberately two-stage. First, `KvmBackend` submits a zero-capacity `KvmMsrList<0>` probe. KVM writes the required count back into `nmsrs` before returning `E2BIG` when the supplied capacity is insufficient, so `E2BIG` is accepted only for this probe. The reported count must be in `1..=1024`, where 1024 is this project's defensive bound rather than a claim about a Linux ABI maximum. The backend then issues a second query with a fixed 1024-entry `repr(C)` buffer and independently validates the final returned count before any Rust slice is formed.

`KVM_GET_MSR_FEATURE_INDEX_LIST` is a separate capability-discovery contract guarded by `KVM_CAP_GET_MSR_FEATURES`. It reports the feature-MSR indices that may be supplied to the system-level `KVM_GET_MSRS` interface for host/KVM feature probing. It uses the same zero-capacity probe and fixed 1024-entry second query, but its validated count is `0..=1024`: an empty feature list is a valid capability snapshot and is not treated as malformed. The second returned count is independently checked before slicing just like the general list.

Validated raw indices are copied into distinct owned typed snapshots. Both lists reuse `MsrIndex`, and both normalize repeated kernel indices by keeping the first occurrence while preserving the kernel's reported order for retained entries. `HostMsrIndexList` represents the general MSR access set; `HostMsrFeatureIndexList` represents the system feature-probing set. Keeping these types separate prevents the two kernel contracts from becoming interchangeable merely because their wire representation is identical. Pure tests lock order preservation, duplicate normalization, typed reuse, and the valid empty feature-list case. Raw variable-length KVM buffers do not escape the KVM module.

The backend then reads the normalized feature indices through the system form of `KVM_GET_MSRS`. `KvmMsrs<N>` models the exact variable-length UAPI as an 8-byte `nmsrs`/padding header followed by 16-byte `KvmMsrEntry` values. Its constructor zeroes the header padding and every entry's reserved/data fields. Before the unsafe ioctl boundary, the wrapper independently requires the requested `nmsrs` to fit the actual backing array, so a malformed userspace header cannot authorize a kernel copy beyond the fixed Rust object.

`KVM_GET_MSRS` returns the number of entries successfully processed rather than rewriting `nmsrs` as a completion count. The backend therefore requires the returned count to equal the complete requested feature-index count. A partial result is rejected as malformed host-discovery state and identifies the first unread feature index when one exists; a returned count greater than requested is also rejected. Before any data becomes typed state, each returned entry index must still equal the requested index at the same position. Only a complete index-stable response becomes owned `MsrFeatureValue` entries inside `HostMsrFeatureValues`. An empty feature-index snapshot produces an empty value snapshot without issuing the value ioctl.

Every `MsrFeatureValue` now carries an `MsrFeatureStability` assigned inside its crate-private constructor, so callers cannot attach inconsistent stability metadata. Linux KVM treats feature MSRs as immutable once the vCPU model is defined except for `MSR_IA32_UCODE_REV`, which tracks the currently loaded microcode patch. The exact architectural index `0x8b` is therefore classified `HostMutable`; every other value returned through the current KVM feature-MSR contract is classified `ModelImmutable`. `HostMsrFeatureValues::model_immutable_values` and `host_mutable_values` expose order-preserving, mutually exclusive views over the same owned snapshot, and focused tests lock the `0x8b` constant plus partition behavior.

`ModelImmutable` is deliberately a narrow KVM-model statement, not a migration guarantee. It means KVM treats the feature value as immutable after the vCPU model is defined; it does not claim that two hosts, kernel versions, CPU revisions, or VMM configurations will expose the same value. `HostMutable` makes the microcode-revision exception explicit so it cannot be silently consumed as though it belonged to an immutable model-capability set.

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

### Full guest MSR snapshot

`GuestMsrSnapshot` is the owned full-policy state boundary layered on top of the more general subset-capable `GuestMsrValueSet`. Its constructor is crate-controlled and accepts one `GuestMsrAccessPolicy` plus one value set produced by the capture path; external callers cannot directly label an arbitrary strict subset as a complete snapshot.

Snapshot construction requires exact coverage and exact positional identity. The number of values must equal the number of policy entries, and every value index must equal the policy index at the same position. Missing, extra, or reordered values are rejected before a snapshot is published. Empty policy plus empty values is a valid full snapshot.

A successful snapshot clones and owns both the complete policy and complete value set. The source policy and capture result may be dropped while the snapshot retains their exact order, authority records, indices, and values. Construction is pure Rust and issues no `KVM_GET_MSRS` or `KVM_SET_MSRS`; it only certifies that the owned value set is a complete ordered image of that specific policy.

`Vcpu::capture_msr_snapshot` is the public vCPU entry point for obtaining this type. It first delegates to the existing policy-bound `Vcpu::capture_msrs`, then passes that fully validated result through the crate-controlled snapshot constructor. It therefore adds no new raw ioctl and preserves the existing all-or-nothing readback semantics. The resulting snapshot is still only one captured MSR state fragment; it is not a migration-stability or restore-success guarantee.

`GuestMsrSnapshot::compare` is a pure reference-to-observed comparison over two already-valid full snapshots. It first compares the complete owned `GuestMsrAccessPolicy` values. If the policies differ, `policy_matches` is false and value-level findings remain empty rather than attempting a positional comparison between different authorization shapes. If the policies match exactly, the full-snapshot invariant guarantees equal coverage and positional index identity, so comparison reports only data differences as `GuestMsrSnapshotValueMismatch` records carrying position, index, reference value, and observed value.

`GuestMsrSnapshotComparison` owns clones of both complete snapshots. `is_exact_match` means only that the policies match and no same-policy value mismatch exists; it does not imply migration safety, cross-host validity, or successful restore semantics. Focused regressions lock identical, empty, policy-mismatch, value-mismatch, and ownership behavior.

### vCPU MSR writes

`Vcpu::set_msrs` is the direct guest-MSR write entry point. It accepts an already-validated `GuestMsrValueSet`, not an arbitrary tuple slice, so callers cannot bypass the `HostMsrIndexList → GuestMsrAccessPolicy → GuestMsrValueSet` authorization path at the write call itself. The write primitive does not derive state from `HostMsrFeatureValues`, `HostMsrModelCandidate`, `CpuModelCandidate`, or `VcpuMsrValues`.

An empty value set returns success without issuing `KVM_SET_MSRS`. A non-empty set is serialized into a fresh zero-initialized `KvmMsrs<1024>` buffer in exact value-set order. Only the active entries' `index` and `data` fields are filled; the header `pad`, each entry `reserved` field, and unused entries remain zero. The vCPU layer rejects more than 1024 values before the ioctl, and `sys::set_msrs` independently validates the encoded `nmsrs` against its actual const-generic backing capacity immediately before the unsafe call.

KVM's successful return is the number of entries processed, not an atomic transaction status. Exact completion is success. A short return becomes `HostEnvironmentError::VcpuMsrPartialWrite` containing the vCPU id, requested count, processed count, and the first unwritten MSR index. The already-processed prefix may have mutated architectural state, so the VMM does not retry, roll back, or describe the failure as atomic. A returned count greater than requested becomes `VcpuMsrInvalidWriteCompletion` and is rejected as malformed completion metadata.

Policy authorization still does not guarantee per-value acceptance. KVM may reject a write because of reserved bits or other architectural constraints even when the index is authorized. This primitive performs one bounded write attempt only; it does not automatically read back state, sequence multiple state classes, or claim migration safety.

`Vcpu::restore_msr_snapshot` is the snapshot-bound write entry point. It accepts an already-certified `GuestMsrSnapshot` and delegates exactly once to `Vcpu::set_msrs(snapshot.values())`; it does not rebuild, revalidate, reorder, or reinterpret the snapshot's policy/value coverage. Empty snapshots therefore reuse the existing empty-write no-op, while non-empty snapshots preserve the captured value order exactly.

Restore inherits the same non-transactional completion semantics as `set_msrs`. A short write propagates `VcpuMsrPartialWrite` unchanged, including the warning that the processed prefix may already have mutated the target vCPU. The restore boundary does not retry or roll back.

`Vcpu::restore_and_verify_msr_snapshot` adds bounded verification without changing those write semantics. It first delegates exactly once to `restore_msr_snapshot`; only after exact write completion does it recapture the same policy through `capture_msr_snapshot(snapshot.policy())`, then returns the existing owned `GuestMsrSnapshotComparison`. A write failure, including `VcpuMsrPartialWrite`, propagates unchanged and prevents readback. A recapture failure after a successful write propagates without retry or rollback, while a value mismatch remains an explicit comparison report rather than triggering automatic repair.

### vCPU MSR readback and policy-bound capture

`Vcpu::msrs` reads architectural MSR state only from an already-created vCPU descriptor and only for the exact `MsrIndex` slice supplied by its caller. It does not consult `HostMsrFeatureValues`, `HostMsrModelCandidate`, or any implicit model candidate to decide what to read. The KVM-aware regression uses indices from `HostMsrIndexList` only as explicit caller-selected supported inputs; that test choice does not create an API dependency from vCPU state to the host feature snapshot.

An empty caller request returns an empty `VcpuMsrValues` immediately and does not issue `KVM_GET_MSRS`. Non-empty requests use a fixed `KvmMsrs<1024>` buffer. The readback layer rejects a caller count above 1024 before the ioctl, while `sys::get_msrs` independently rechecks the encoded `nmsrs` against its actual const-generic backing capacity immediately before the unsafe call.

Request construction copies every caller index into the KVM entries in the same order and deliberately does not normalize duplicates. KVM's successful return value is treated as an untrusted completion count: it must equal the requested count exactly. A partial completion reports the first unread requested index; an impossible over-completion is rejected. The response entry slice must also contain the complete requested prefix, and every returned entry index must equal the caller's index at the same position before any typed values are published.

Only a fully completed, position-stable response becomes owned `VcpuMsrValue` entries inside `VcpuMsrValues`. These types contain only `MsrIndex` plus the architectural `u64` value and intentionally carry no `MsrFeatureStability`, because system feature-MSR stability classification is not guest vCPU-state metadata. The result owns its values and contains no pointer or borrow into the vCPU descriptor or KVM request buffer.

`Vcpu::capture_msrs` is the policy-bound capture path. It accepts one already validated `GuestMsrAccessPolicy`, extracts exactly those policy indices in policy order, and delegates the actual read to `Vcpu::msrs`; it adds no raw ioctl and no second KVM decoder. An empty policy returns an empty `GuestMsrValueSet` before entering the generic readback path, so it issues no `KVM_GET_MSRS`.

Capture is deliberately stricter than the general subset semantics of `GuestMsrValueSet`. Before materialization, the capture layer independently requires the typed readback length to equal the complete policy length and every returned index to match the corresponding policy entry. Only then are the index/value pairs passed through `GuestMsrValueSet::from_policy`. A partial prefix, changed index, or reordered internal capture is rejected rather than published as a valid subset. Focused pure regressions lock policy-order extraction, exact value transfer, empty behavior, partial-prefix rejection, and index mismatch; the KVM-aware regression locks the public policy-bound path when `/dev/kvm` is available.

`Vcpu::capture_msr_snapshot` builds on that path rather than introducing another read contract. After a complete policy-bound capture succeeds, it validates and owns the exact policy/value pairing as `GuestMsrSnapshot`. `Vcpu::verify_msr_snapshot` performs exactly one fresh snapshot capture through the reference snapshot's own policy and returns the existing pure comparison; it does not call a setter, restore, retry, repair, or rollback. Readback, value-set materialization, full-snapshot certification, pure snapshot comparison, read-only verification, direct write, snapshot-bound restore, and restore verification therefore remain separate typed boundaries.

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

`Vcpu::capture_register_snapshot` performs one existing `KVM_GET_REGS` and copies all 18 x86 general-register fields (RAX through R15, RIP, and RFLAGS) into an owned `VcpuRegisterSnapshot`. `VcpuRegisterSnapshot::compare` performs deterministic pure reference-to-observed comparison over those 18 fields. `Vcpu::verify_register_snapshot` performs one fresh capture and returns that existing comparison without invoking `KVM_SET_REGS`, restore, retry, repair, or rollback. Snapshot-bound restore/restore-and-verify continue to use the validated KVM register setters/readback without claiming transactionality or rollback. The existing `Vcpu::registers` API remains the lighter RIP/RFLAGS diagnostic view and is not changed by snapshot capture.

Special-register capture likewise owns semantic x86 segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM padding. Pure comparison and `verify_special_register_snapshot` remain read-only; the latter performs one fresh capture and returns the existing comparison without restoring a mismatch. Snapshot-bound restore and restore-and-verify preserve the existing typed write boundary. `VcpuStateSnapshot` composes general-register, special-register, and policy-bound MSR snapshots; its comparison preserves component reports, `verify_state_snapshot` performs a fresh read-only capture, and restore/restore-and-verify sequence the existing component operations with explicitly bounded non-transactional semantics. None of these values is a whole-VM, guest-memory, device-state, checkpoint, migration, atomic/quiesced snapshot, or rollback primitive.

`Vcpu::msrs`, `Vcpu::capture_msrs`, `Vcpu::capture_msr_snapshot`, `Vcpu::verify_msr_snapshot`, `Vcpu::set_msrs`, `Vcpu::restore_msr_snapshot`, and `Vcpu::restore_and_verify_msr_snapshot` are synchronous state operations over the same owned vCPU descriptor used by the execution path. The current VMM has no scheduler or concurrent vCPU runner; callers invoke these primitives directly rather than racing an independently executing vCPU thread. Readback returns owned typed values, policy-bound capture returns an owned validated value set, full snapshot capture returns an owned policy/value binding, read-only verification performs one snapshot-bound recapture and pure comparison, direct writes consume only an owned policy-validated value set, snapshot restore delegates the snapshot's owned value set to the same write boundary, and restore verification sequences one restore with one recapture before returning the existing owned comparison report. None retains a caller borrow after its operation returns.

`Vcpu::run_once` retries an interrupted host syscall, performs one completed `KVM_RUN`, reads the tested x86 prefix of `kvm_run`, and returns a typed `VcpuExit`. HLT, port I/O, legacy `KVM_EXIT_SHUTDOWN`, `KVM_EXIT_FAIL_ENTRY`, `KVM_EXIT_INTERNAL_ERROR`, and `KVM_EXIT_SYSTEM_EVENT` are classified explicitly; unknown reasons retain the exact raw reason.

For `KVM_EXIT_IO`, `Vcpu::port_io_exit` is the only layer allowed to inspect the I/O union and referenced data area. It validates direction, converts and checks `data_offset`, computes `size * count` with checked arithmetic, validates the complete range against the mmap length, and copies OUT bytes into owned Rust memory. IN requests expose metadata but no borrowed mmap data. No pointer into `kvm_run` leaves the vCPU layer.

For `KVM_EXIT_IO_IN`, `Vcpu::write_port_io_input` re-reads the current I/O metadata, requires IN direction, recomputes the complete checked mmap range, and requires the response length to equal that range exactly before copying the owned bytes into `kvm_run`.

For `KVM_EXIT_FAIL_ENTRY`, `Vcpu::fail_entry` is the only layer allowed to form the tested fixed x86 fail-entry prefix view. The decoder first requires the current exit reason to be fail-entry, then copies KVM's raw `hardware_entry_failure_reason` and `cpu` fields into owned `VcpuFailEntry` state without interpreting those architecture-specific values as recovery policy. The larger common `kvm_run` prefix-size requirement already covers the fail-entry view, and no raw pointer or borrowed payload crosses into dispatch.

For `KVM_EXIT_INTERNAL_ERROR`, `Vcpu::internal_error` is the only layer allowed to inspect the tested x86 internal-error union. The always-available base view begins at union offset 32 and ends after `suberror: u32`; when the inherited optional-support flag is false, decoding forms only that base view, copies `suberror`, and exposes `VcpuInternalError::data()` as `None`. When the vCPU inherited a positive `KVM_CAP_INTERNAL_ERROR_DATA` observation, decoding may instead form the fixed full 168-byte prefix containing `suberror`, `ndata`, and `data[16]`. Kernel `ndata` is untrusted and must be `<= 16` before any slice is formed; only the declared words are copied into owned `VcpuInternalError` state. `ndata == 0` is represented as available-but-empty data (`Some(&[])`), which is distinct from capability absence. An out-of-capacity count becomes structured `InvalidInternalErrorDataCount` diagnostics retaining vCPU id, `suberror`, reported `ndata`, capacity, and later the execution trace. No raw pointer or borrowed internal-error payload crosses into dispatch, and no optional payload value is interpreted as recovery policy.

For `KVM_EXIT_SYSTEM_EVENT`, `Vcpu::system_event` is the only layer allowed to form the tested 168-byte x86 system-event prefix view. Mapping construction requires the shared region to be large enough before that cast can be formed. The decoder requires the current exit reason to be system-event, validates the kernel-reported `ndata` against the fixed 16-word UAPI capacity before slicing, preserves the current raw event-type values (including unknown values), and copies only the declared payload words into an owned `VcpuSystemEvent`. No raw pointer or borrowed system-event data crosses into dispatch.

## Bounded execution loop

`execution::run_vcpu_until_stopped` is the single reusable run-loop boundary for the current one-vCPU model. Before each `KVM_RUN` it checks an explicit completed-exit budget. A successful `KVM_RUN` consumes exactly one budget unit; host-side failures that do not produce a completed VM exit consume none.

Each completed exit is recorded exactly once in an ordered raw reason trace before dispatch. Serviceable I/O is recorded as an owned `PortIoExit` and execution continues while budget remains. A terminal HLT or legacy shutdown returns `VmExecutionResult`, which contains the terminal `VmExitReport`, every serviced typed I/O exit, the exact completed-exit count, and the complete ordered raw reason trace.

A zero budget fails before any guest run. When the configured budget is exhausted, the next run attempt fails with `VmExitError::ExitBudgetExhausted`, preserving vCPU id, configured limit, completed count, last completed raw exit reason when available, and the complete ordered trace of completed exits. Exhaustion is not reported as guest termination. If the final permitted exit was serviceable port I/O, userspace may have prepared the service response but the VMM does not claim the pending KVM operation completed because no further `KVM_RUN` was permitted.

Unhandled raw exits, fail-entry diagnostics, internal-error diagnostics (including invalid optional-data counts), and system-event diagnostics are also annotated with the complete ordered trace accumulated by the loop. A fail-entry reason `9`, internal-error reason `17`, or system-event reason `24` appears exactly once at the tail because the successful `KVM_RUN` is recorded before payload decoding/dispatch. This diagnostic ownership does not create resumable execution or partial-success result semantics.

The HLT and CPUID fixtures use budget 1. Both deterministic port fixtures use budget 2, so their successful sequence is exactly one serviceable I/O exit followed by terminal HLT. Extra serviceable exits cannot be silently accepted: they consume the budget and prevent a terminal success report.

## VM-exit dispatch

`vmexit::dispatch_vcpu_exit` is the single policy boundary for one completed vCPU exit.

- HLT and legacy `KVM_EXIT_SHUTDOWN` snapshot RIP/RFLAGS and become `VmExitDisposition::Stopped(VmExitReport)`.
- Port I/O is parsed into an owned `PortIoExit` and routed through `PortIoBus`.
- An OUT service records/captures device output and becomes `Continue` without writing the run mapping.
- An IN service returns owned response bytes; the dispatcher asks the vCPU layer to validate and write those bytes into the pending KVM input range before returning `Continue`.
- `KVM_EXIT_FAIL_ENTRY` is decoded into owned hardware-entry-failure/CPU diagnostics and returned as structured `VmExitError::EntryFailure` without issuing `KVM_GET_REGS` or another vCPU ioctl that could obscure the original entry failure.
- `KVM_EXIT_INTERNAL_ERROR` is decoded into owned `suberror` plus capability-gated optional data and returned as structured `VmExitError::InternalError` without issuing `KVM_GET_REGS` or another secondary vCPU ioctl; malformed capability-enabled `ndata` becomes `VmExitError::InvalidInternalErrorDataCount`. Neither path adds suberror/data interpretation, recovery, retry, replacement execution, or lifecycle policy.
- `KVM_EXIT_SYSTEM_EVENT` is decoded into owned event type/data, paired with register diagnostics, and returned as structured `VmExitError::UnsupportedSystemEvent`; no shutdown/reset/crash/wakeup/suspend/SEV-termination/TDX-fatal lifecycle action is implemented.
- Unsupported raw exit reasons become `VmExitError::Unhandled` with vCPU id and register diagnostics.

Legacy `KVM_EXIT_SHUTDOWN` reason `8` is intentionally distinct from `KVM_EXIT_SYSTEM_EVENT` reason `24` carrying event type `Shutdown`. The former is the existing typed terminal stop; the latter remains a structured unsupported event until an explicit lifecycle policy is designed.

The dispatcher deliberately does **not** snapshot registers for an in-flight KVM I/O exit. KVM defines port-I/O operations as pending until userspace re-enters `KVM_RUN`; register state used as a completed-operation diagnostic is therefore taken only on a later terminal exit. Fail-entry and internal-error diagnostics likewise deliberately avoid a secondary register read: the purpose-built payload from the completed exit is preserved directly rather than risking replacement by a subsequent ioctl failure.

The deterministic output fixture reaches HLT at RIP `0x1005`. The deterministic input fixture receives byte `R`, re-enters KVM so KVM transfers that byte into AL, executes `MOV [0x2000], AL`, and reaches HLT at RIP `0x1006`. The CPUID fixture has no userspace-serviced exits and reaches HLT at RIP `0x101c`; host code then reads its two checked result words.

## Port-I/O bus and debug device

`PortIoBus` is intentionally minimal. It may contain one exact debug-port device at port `0xe9`; it is not yet a general dynamic device registry or port-range resolver.

The debug device accepts only byte-wide, single-count accesses at `0xe9`:

- OUT requires a copied payload length of exactly 1 byte and appends that byte to the device output buffer.
- IN returns exactly one configured byte as owned response data.

Unknown ports become `PortIoError::UnhandledPort`. Wider or multi-count operations to `0xe9` become `PortIoError::UnsupportedDebugAccess`. An OUT payload-length mismatch is explicit. The vCPU layer independently rejects an IN response whose length does not exactly match the checked KVM data range. No request is silently truncated, widened, repeated, or redirected.

## Ownership and lifetime

`KvmBackend` owns `/dev/kvm`, validated required host capabilities plus the optional `KVM_CAP_INTERNAL_ERROR_DATA` observation, the typed `HostCpuid`, `HostMsrIndexList`, `HostMsrFeatureIndexList`, and stability-annotated `HostMsrFeatureValues` discovery snapshots, and the derived `GuestCpuPolicy`. `GuestMsrAccessPolicy` is an on-demand owned derivative of the validated general MSR capability set plus explicit caller authorization; it keeps no borrow into `KvmBackend` or the caller slice. `GuestMsrValueSet` is another on-demand owned derivative of an access policy plus explicit caller state; it owns only typed MSR index/value pairs and keeps no borrow into the policy or caller slice. `GuestMsrSnapshot` is an owned full-policy capture derivative that clones one complete access policy plus the exact complete ordered value set validated against it; it keeps no borrow into either source value. `GuestMsrSnapshotComparison` owns clones of both complete snapshots plus same-policy value-mismatch findings, so comparison survives after either source snapshot is dropped. `GuestCpuPolicyComparison` is an on-demand owned derivative that clones both complete configured policies plus all directional findings; it holds no borrow into backend or VM state. `HostMsrModelCandidate` is an on-demand owned derivative: it clones the complete feature-value observation as provenance and separately owns only its immutable subset. `HostMsrModelComparison` is also owned: it clones both complete candidates and owns its missing/extra/value-mismatch findings, so comparison data carries no borrow into backend state. `CpuModelCandidate` is another owned derivative that clones one complete configured `GuestCpuPolicy` plus one complete `HostMsrModelCandidate`; the latter continues to own its complete source-observation provenance. `CpuModelComparison` owns the resulting `GuestCpuPolicyComparison` and `HostMsrModelComparison`, so the composed report has no borrow into either source candidate and retains both component provenance chains. `Vm` owns the VM descriptor, a clone of the guest CPUID policy, the inherited optional internal-error-data support flag, and its optional registered guest RAM. CPUID read-back buffers and decoded comparison entries are temporary data inside vCPU construction; neither is retained after exact verification. `Vcpu` owns the vCPU descriptor, `KvmRunMapping`, and the inherited optional internal-error-data support flag; register, special-register, MSR, and composite state captures return separate owned typed snapshots and comparisons; read-only verification methods borrow only their reference snapshots for one fresh capture plus pure comparison, while restore methods borrow only their typed snapshot inputs for the duration of the bounded setter/readback sequence. `VcpuFailEntry` owns the raw hardware entry failure reason and CPU field copied from `kvm_run`; `VcpuInternalError` owns the raw internal-error `suberror` plus a fixed owned optional-data buffer/count when that payload is capability-enabled, exposing `None` when unavailable and an owned-backed slice when available; `VcpuSystemEvent` owns its decoded event type and copied declared data words rather than borrowing the shared mapping. `PortIoBus` owns its optional debug device, configured input byte, and accepted output bytes. `VmExecutionResult`, execution errors carrying completed-exit traces, and `CpuidGuestResult` own only copied safe Rust data and reports; none contains a pointer or borrow into KVM shared memory or guest RAM.

Rust ownership is used for normal cleanup; explicit KVM slot removal protects the guest-RAM lifetime boundary when independent vCPU descriptors exist.

## Error boundary

Errors are categorized as:

- `HostEnvironment`: host file/device/I/O failures, malformed host KVM variable-length responses including CPUID, general MSR-index, MSR-feature-index, system MSR-feature-value completion/index semantics, vCPU MSR read request/response validation, policy-bound vCPU MSR capture materialization validation, full guest MSR snapshot validation at the vCPU capture boundary, vCPU MSR write request/completion validation including structured non-transactional partial writes, snapshot-bound restore and restore-verification propagation of those same write/read diagnostics, named VM/vCPU ioctls including CPUID query/application/read-back plus `KVM_GET_REGS`, `KVM_GET_MSRS`, and `KVM_SET_MSRS`, and CPUID read-back policy mismatches;
- `KvmCapability`: incompatible API version, absent required extension, or invalid kernel-reported mapping size;
- `Configuration`: unsupported VMM configuration or current real-mode entry limits;
- `GuestMemory`: invalid guest ranges, reserved-range overlap, mapping failures, bounds violations, or KVM RAM-registration failures;
- `GuestImage`: malformed or overflowing flat-image descriptions;
- `VmExit`: unsupported raw exits, fail-entry payload-unavailable/entry-failure diagnostics, internal-error payload-unavailable/capability-aware diagnostics including invalid optional-data counts, unsupported system events, unavailable/malformed system-event payload metadata, malformed KVM I/O metadata/ranges, invalid IN response direction/length, execution-budget exhaustion, or deterministic fixture sequence failures;
- `PortIo`: unknown ports or unsupported/malformed device accesses.

Pure guest-MSR policy construction has its own `GuestMsrPolicyError`, while pure value-set materialization has `GuestMsrValueSetError`. Crate-controlled full-snapshot construction has `GuestMsrSnapshotError`, used to reject coverage or positional-index mismatch before a `GuestMsrSnapshot` can exist. Snapshot comparison itself is total over already-valid snapshots and therefore introduces no new error type. Unsupported/duplicate caller authorization and unauthorized/duplicate caller value state are configuration/state validation failures and do not originate from host I/O or a vCPU operation. Policy-bound capture uses the existing vCPU-operation error boundary if its independently checked full-policy materialization invariant is violated. `VcpuMsrPartialWrite` is intentionally different: it reports that a kernel write attempt stopped after a prefix that may already have changed vCPU state, and snapshot restore preserves that diagnostic unchanged. Future MMIO, interrupt, whole-VM/device-snapshot, and stronger invariant categories will be added only when those responsibilities exist.

## Deliberate non-abstractions

There is no generic hypervisor backend trait yet. KVM is the only implementation, and an abstraction would not have a second consumer. The KVM-specific plumbing is nevertheless isolated so a later raw-VMX research backend would not require leaking ioctls into VM policy.

There is no configurable or migration-stable CPU model yet. The current boundary distinguishes discovered host support from a derived guest CPUID policy and immutable host MSR model candidate, then allows them to be composed and compared without turning an exact component match into a named cross-host compatibility guarantee. The current CPUID policy remains host-derived with conservative masking for the absent LAPIC model.

The implemented state lifecycle is deliberately vCPU-CPU-state scoped. General-register, special-register, policy-bound MSR, and composite `VcpuStateSnapshot` values support owned capture, pure comparison, and snapshot-bound read-only verification; the register/MSR/composite paths also expose bounded restore and restore-and-verify. These operations are synchronous, non-transactional across multi-component restore, and do not imply a quiesced atomic point in time. There is still no automatic mismatch repair, no rollback, no multi-vCPU restore orchestration, no guest-memory/device snapshot, no checkpoint decoder, and no migration protocol.

`KVM_EXIT_FAIL_ENTRY` now has typed classification and owned purpose-built diagnostics, but there is deliberately no retry, CPU-affinity/placement, hardware-failure-bit interpretation, or recovery policy. Adding such behavior requires an explicit execution/recovery design rather than interpreting opaque architecture-specific fields heuristically.

`KVM_EXIT_INTERNAL_ERROR` now has typed classification and capability-gated optional payload diagnostics. The always-available raw `suberror` is copied on every internal-error exit. The backend separately queries optional `KVM_CAP_INTERNAL_ERROR_DATA` and propagates only positive support into created vCPUs; hosts without support remain valid and use the base-only path. Capability-enabled decoding validates `ndata <= 16` before slicing and owns only the declared `data[16]` prefix, with empty declared data kept distinct from unavailable data. This richer diagnostic boundary still deliberately provides no suberror/data-specific emulation recovery, retry, replacement execution, or lifecycle policy.

`KVM_EXIT_SYSTEM_EVENT` now has typed classification and owned payload diagnostics, but there is deliberately no reset/reboot/crash/shutdown/wakeup/suspend/SEV-termination/TDX-fatal lifecycle policy. Adding such policy requires an explicit VM lifecycle design rather than treating the decoded event as an implicit terminal action.

There is also no multi-region memory map yet. `GuestMemoryRegion::overlaps` exists to make range semantics explicit and tested, but the VM intentionally supports only slot 0 in this milestone.

The port bus is not a trait-object registry yet. One exact bidirectional device is enough to prove OUT copying and IN response write-back without introducing registration/range-resolution machinery prematurely.

The execution loop is not a scheduler. It owns no vCPU, thread, timer, or interrupt state; it only bounds repeated execution of one already-created vCPU.

## Next architectural milestone

No architectural milestone is selected in this document. `ROADMAP.md` is the authoritative live source for next-slice selection; after each integrated slice and exact post-merge CI result, future work must be chosen from the then-current repository state rather than from a historical architecture note.
