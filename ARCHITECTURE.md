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
       │   Elf64GuestImage
       │       ├─ checked ELF64 ET_EXEC/PT_LOAD materialization + BSS zeroing
       │       └─ validated bounded RAM-backed virtual→physical alias mapping plan
       │   LongModeBootLayout
       │       ├─ preserved fixed 2 MiB identity map
       │       └─ optional checked RAM-backed 4 KiB mappings in fixed alias window
       │   LongModeMmioBootLayout
       │       └─ one explicit device PTE: VA 0x500000 → unbacked GPA 0x10000000
       │   LongModeInterruptLayout
       │       ├─ base identity long-mode layout
       │       ├─ fixed GDT page at GPA 0x5000
       │       └─ fixed IDT page at GPA 0x6000 with vector 0x40 gate
       └─ vCPU creation
              ├─ KVM_CREATE_VCPU
              ├─ GuestCpuPolicy → KVM_SET_CPUID2
              ├─ bounded KVM_GET_CPUID2 → typed exact verification
              ├─ propagate optional internal-error-data support
              ↓
             Vcpu
              ├─ explicit real-mode register setup for legacy/MMIO fixtures
              ├─ explicit x86-64 long-mode sregs/regs setup for long-mode/ELF64/virtual-MMIO fixtures
              ├─ interrupt layout → long-mode setup + exact GDT/IDT sregs
              ├─ request interrupt window → IRQ_WINDOW_OPEN + ready/IF → KVM_INTERRUPT
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
              ├─ checked KVM_EXIT_MMIO payload extraction
              ├─ checked KVM_EXIT_MMIO read-response write-back
              ├─ checked KVM_EXIT_UNKNOWN hardware-diagnostic extraction
              ├─ checked KVM_EXIT_EXCEPTION payload extraction
              ├─ checked KVM_EXIT_FAIL_ENTRY payload extraction
              ├─ capability-gated KVM_EXIT_INTERNAL_ERROR payload extraction
              │    └─ raw suberror → VcpuInternalErrorSuberror (pure, lossless classification)
              ├─ checked KVM_EXIT_SYSTEM_EVENT payload extraction
              └─ KVM_RUN → VcpuExit
                         ↓
       execution::run_vcpu_until_stopped / run_vcpu_until_stopped_with_mmio
              ├─ explicit completed-exit budget
              ├─ ordered completed-exit reason trace
              ├─ records serviced typed port-I/O and MMIO exits
              └─ vmexit::dispatch_vcpu_exit
                   ├─ HLT / legacy shutdown → VmExitReport → stop
                   ├─ IO → PortIoBus → debug port 0xe9 → continue
                   ├─ MMIO → MmioBus → exact configured byte-device GPA → continue
                   ├─ KVM_UNKNOWN → structured hardware diagnostic
                   ├─ EXCEPTION → structured exception diagnostic
                   ├─ FAIL_ENTRY → structured entry-failure diagnostic
                   ├─ INTERNAL_ERROR → structured capability-aware diagnostic
                   ├─ SYSTEM_EVENT → structured unsupported diagnostic
                   └─ other unsupported raw reason → VmExitError::Unhandled
```

The raw ioctl UAPI details live in `src/kvm/sys.rs`; tested `kvm_run` payload views stay isolated below the vCPU layer. Higher layers call typed Rust methods and do not issue raw `ioctl` operations or inspect raw shared-memory payload layouts directly. The direct interrupt path adds one fixed `kvm_interrupt { irq: u32 }` request through that same UAPI boundary; before that request, the vCPU layer uses only the fixed `KvmRunHeader` fields `request_interrupt_window`, `ready_for_interrupt_injection`, and `if_flag` to establish KVM's injection-ready state. The `irq` field is treated as an x86 interrupt vector, not as a PIC/APIC pin or routing object.

## x86 host and CPU capability contract

The backend requires KVM API version 12 plus `KVM_CAP_USER_MEMORY`, `KVM_CAP_SET_TSS_ADDR`, `KVM_CAP_EXT_CPUID`, `KVM_CAP_SET_IDENTITY_MAP_ADDR`, and `KVM_CAP_GET_MSR_FEATURES`.

Separately from that required set, `KvmBackend` queries `KVM_CAP_INTERNAL_ERROR_DATA` (capability ID 40) through the same `KVM_CHECK_EXTENSION` boundary and stores the raw returned value in the existing owned `HostCapabilities.extensions` snapshot. A missing observation in manually constructed `HostCapabilities` or a recorded value of `0` does not invalidate an otherwise valid host; `internal_error_data_capability()` exposes the recorded observation when present and `supports_internal_error_data()` is true exactly when its raw value is greater than zero. `KvmBackend::create_vm` propagates only that support boolean into `Vm`, and each created `Vcpu` inherits the same fact. A non-positive observation keeps internal-error decoding on the always-available base `suberror` view. A positive observation authorizes the vCPU decoder to form the fixed full x86 internal-error payload view and validate/copy declared optional data; it still does not create emulation recovery, retry, replacement execution, or lifecycle policy. Typed suberror classification is independent of this capability.

After fixed host capability validation, `KvmBackend` performs `KVM_GET_SUPPORTED_CPUID` through a fixed 256-entry `repr(C)` `KvmCpuid2<N>` buffer. The kernel-returned `nent` is not trusted as a Rust slice length: it must be non-zero and no greater than the fixed capacity before the entry prefix is converted into owned `CpuidEntry` values. Reserved KVM padding is absent from the typed representation, and conversion back to KVM UAPI always writes zero padding.

Host discovery and guest policy are separate states. `HostCpuid` preserves the validated KVM-supported function/index/flags/register values exactly; `GuestCpuPolicy::from_host` clones that snapshot and applies the current no-LAPIC policy as a pure transformation. The current interrupt-controller model still has no in-kernel LAPIC or IRQ chip, so the guest policy removes x2APIC, TSC-deadline, and `KVM_FEATURE_PV_UNHALT` and synthesizes no additional feature. The bounded direct-vector path deliberately leaves that policy unchanged: proving `KVM_INTERRUPT` delivery does not establish local-APIC, IOAPIC, timer, interrupt-priority, or IRQ-routing semantics.

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

Those KVM-reserved pages are distinct from the guest-created long-mode page tables and direct-interrupt descriptor tables at low guest physical addresses. Guest RAM registration rejects any region overlapping the high KVM-reserved range. The direct-interrupt milestone does not call `KVM_CREATE_IRQCHIP`; the VM therefore remains in the existing userspace interrupt-controller mode.

## Guest memory

`GuestPhysAddr` distinguishes guest physical addresses from host pointers. `GuestMemoryRegion` owns checked range semantics; `GuestMemory` owns the anonymous host mapping and performs guest-address validation before host memory copies. The current implementation accepts exactly one page-aligned, non-zero RAM region and registers it as KVM slot 0.

The region constructor rejects guest-physical wraparound and alignment errors. Access validation rejects address-plus-length overflow, ranges outside RAM, and host-size conversion failures. Zero-length accesses are valid at the exclusive end; non-zero accesses are not.

The `Vm` takes ownership of `GuestMemory` only after `KVM_SET_USER_MEMORY_REGION` succeeds. During `Vm` destruction it first issues a zero-sized slot-0 update to unregister RAM. If KVM refuses that cleanup, the process intentionally leaks the backing mapping rather than unmapping memory while a surviving vCPU fd could still keep the kernel VM alive.

The real-mode MMIO proof deliberately registers a fresh 4 KiB slot-0 RAM region `0..0x1000` and accesses fixed GPA `0x2000`, leaving that address unbacked in that VM so KVM exits to userspace. This is fixture-relative rather than a global memory-map reservation: other fresh fixtures use `0x2000` as ordinary RAM or as the long-mode PDPT page.

The long-mode virtual-MMIO proof uses the normal 2 MiB slot-0 RAM layout but maps data VA `0x500000` to fixed GPA `0x10000000`. `LongModeMmioBootLayout` rejects construction if the configured RAM exclusive end extends beyond that GPA, so the translated device page remains unbacked rather than becoming ordinary guest RAM.

The direct-interrupt proof also uses the normal 2 MiB slot-0 RAM layout. `LongModeInterruptLayout` reserves guest-memory GDT and IDT pages at `0x5000` and `0x6000` in addition to the existing page-table extent `0x1000..0x5000`. The exact vector-`0x40` IDT gate is stored at `0x6400`; handler code is identity-mapped at `0x11000`. These pages remain ordinary registered guest RAM owned by the VM, not KVM irqchip state.

See [docs/memory-map.md](docs/memory-map.md).

## Guest loading

`FlatGuestImage` remains the minimal raw-image path. Construction requires a non-empty byte slice, rejects load-address overflow, and requires the entry point to lie inside the loaded image. Loading goes through `GuestMemory::write`, so a valid flat image description cannot escape the configured RAM region.

The existing HLT, debug-port output, debug-port input, and CPUID fixtures remain reviewed real-mode flat binaries at entry `0x1000`. The real-mode MMIO fixture is also a reviewed flat binary but uses entry `0x100` inside its dedicated 4 KiB RAM region so absolute address `0x2000` remains unbacked. The original long-mode proof, long-mode virtual-MMIO proof, and direct-interrupt main path remain flat binaries at GPA/VA `0x10000`; the direct-interrupt fixture additionally loads its handler at GPA/VA `0x11000`.

### Bounded ELF64 executable loading

`loader::elf64::Elf64GuestImage` is a separate, deliberately bounded executable-format boundary. It accepts only ELF64, little-endian, x86-64 `ET_EXEC`. The fixed ELF header and complete program-header table are validated before program headers are traversed or file-backed slices are formed.

Every `PT_LOAD` must have non-zero `p_memsz`, `p_filesz <= p_memsz`, a checked file range, independently checked virtual and physical ranges, and alignment 0/1 or a power of two with the required file-offset/virtual-address congruence. Physical backing must remain inside the low `0..0x20_0000` guest RAM and outside bootstrap page tables `0x1000..0x5000`. Loadable segments may not overlap in either virtual address space or physical backing space. At least one loadable segment is required.

A load segment whose virtual range is inside the low identity map requires `p_vaddr == p_paddr`. A non-identity load segment is accepted only when the complete virtual range is inside the fixed `0x40_0000..0x60_0000` alias window and its virtual/physical addresses have equal 4 KiB page offsets. Parsing derives and owns the required `LongModePageMapping` plan; conflicting alias virtual-page mappings are rejected before page-table installation.

The ELF entry point must lie in the file-backed portion of an executable `PT_LOAD`; an address that exists only in BSS is not executable entry evidence. Loading copies each validated file-backed range to its physical backing with `GuestMemory::write` and explicitly zeroes `p_memsz - p_filesz` physical bytes before VM registration. Virtual addresses are never used as guest-memory write offsets.

The deterministic ELF64 fixture has one executable `PT_LOAD` with virtual base `0x400000`, physical backing base `0x10000`, virtual entry `0x400100`, and a BSS tail within the physical `0x10000..0x10180` backing range. It then reuses `LongModeBootLayout::with_page_mappings`, `Vcpu::initialize_long_mode`, the execution loop, and the debug-port model. The guest emits `LM64` and reaches HLT at virtual RIP `0x400124`. This boundary does not implement relocations, `ET_DYN`/PIE, dynamic linking, section semantics, arbitrary virtual windows, dynamic page-table allocation, or Linux boot conventions.

## x86-64 long-mode bootstrap

`LongModeBootLayout` is the correctness contract for the current 64-bit bootstrap. It is intentionally bounded rather than a general virtual-memory manager.

The layout requires guest RAM to start at GPA `0` and contain at least `0x20_0000` bytes. Four 4 KiB pages are reserved for bootstrap translation state:

- PML4 at GPA `0x1000`;
- PDPT at GPA `0x2000`;
- PD at GPA `0x3000`;
- bounded alias PT at GPA `0x4000`;
- reserved page-table extent `0x1000..0x5000`.

`install_page_tables` zeroes all four pages through checked `GuestMemory::write`, then installs the preserved base chain: PML4[0] = `0x2003`, PDPT[0] = `0x3003`, and PD[0] = `0x83`. The PD[0] entry is a present, writable 2 MiB large page, so virtual addresses `0..0x20_0000` identity-map to the same guest physical addresses. Identity-only layouts leave the alias PDE unlinked.

`LongModeBootLayout::with_page_mappings` accepts additional `LongModePageMapping` values only when every virtual page is 4 KiB aligned and inside `0x40_0000..0x60_0000`, every physical page is 4 KiB aligned and wholly inside the low 2 MiB RAM outside `0x1000..0x5000`, and no virtual page is duplicated. When mappings are present, PD[2] points to GPA `0x4000`; each requested alias page selects one PTE index within that fixed 512-entry table and stores the validated RAM physical page plus present/writable flags. Unused PTEs remain zero. An alias-window entry is accepted only if its containing virtual page is mapped. This RAM-backed mapping contract is unchanged by MMIO or interrupt support.

`LongModeMmioBootLayout` is a separate composition boundary. It first constructs the identity-only `LongModeBootLayout`, requires fixed device GPA `0x10000000` to remain outside slot-0 RAM, then links PD[2] to the same alias PT and writes exactly PTE index 256 so virtual page `0x500000` maps to the unbacked device page. It does not create a `LongModePageMapping`, does not relax its RAM validation, and does not allow caller-selected device pages or virtual addresses.

`LongModeInterruptLayout` is another composition boundary over the identity-only `LongModeBootLayout`. It keeps the same page-table construction and identity mapping, then owns fixed descriptor-table pages at `0x5000` and `0x6000`. The GDT contains a null descriptor plus the ring-0 code/data descriptors matching the established long-mode selectors. The IDT is zeroed except for the vector `0x40` interrupt gate targeting RIP `0x11000` with selector `0x8`. Layout validation rejects exception-reserved vectors, entry/handler collisions with `0x1000..0x7000`, a handler outside the low identity map, and a conservative bounded interrupt stack frame overlapping those reserved pages. It does not accept arbitrary descriptor tables.

`LongModeBootLayout::new` retains the identity-only entry contract. The RAM-backed constructor additionally accepts a mapped alias entry only when the entry's virtual page is present in its validated mapping set. `LongModeMmioBootLayout` keeps executable RIP in the low identity map and uses its device mapping only for data access. `LongModeInterruptLayout` likewise uses identity-mapped main and handler code. All current long-mode fixtures require the stack pointer to remain non-zero inside the low identity map; the interrupt layout additionally validates the bounded frame below that stack pointer. The flat long-mode, virtual-MMIO, and direct-interrupt proofs use identity entry GPA/VA `0x10000`; the ELF64 proof uses virtual entry `0x400100` backed at GPA `0x10100`; all use RSP `0x1ff000`.

`Vcpu::initialize_long_mode` begins from KVM's current special-register state. It preserves unrelated inherited control/EFER bits while requiring `CR0.PE|CR0.PG`, `CR4.PAE`, and `EFER.LME|EFER.LMA`; it writes `CR3 = 0x1000`. CS is a present ring-0 flat 64-bit code segment with selector `0x8`, long bit set, default operand-size bit clear, base zero, and limit `0xffff_ffff`. DS/ES/FS/GS/SS use the fixed present ring-0 data-segment contract with selector `0x10`, base zero, and limit `0xffff_ffff`. The general-register write sets RIP to the validated architectural entry address, RSP to the validated stack pointer, and RFLAGS bit 1 while zero-initializing the remaining general-register fields.

`Vcpu::initialize_long_mode_interrupts` reuses that exact base initialization, then performs one fresh `KVM_GET_SREGS`/`KVM_SET_SREGS` pair to install only the validated GDT and IDT base/limit fields. `Vcpu::wait_for_interrupt_window` sets only `kvm_run.request_interrupt_window`, performs one `KVM_RUN`, clears the request flag, requires `KVM_EXIT_IRQ_WINDOW_OPEN` plus `ready_for_interrupt_injection=1` and guest IF set, then captures RIP/RFLAGS for fixture evidence. Only after that handshake does `Vcpu::inject_interrupt` serialize vector `0x40` into the fixed `kvm_interrupt` structure and issue `KVM_INTERRUPT`. Failures propagate through the existing named hard-error boundary. The path does not call `KVM_CREATE_IRQCHIP`, does not request a pin/line, and does not infer routing semantics.

The deterministic 36-byte flat guest intentionally contains 64-bit-only/64-bit-width instruction encodings (`REX.W` `movabs` and 64-bit shifts). It emits `L`, `M`, `6`, `4` through four byte-wide OUT operations to the existing debug port `0xe9`, then executes HLT. The bounded run therefore completes exactly five exits: four I/O exits followed by HLT. A successful terminal report has RIP `0x10024` and the collected debug output is exactly `LM64`.

The same bootstrap also executes the bounded ELF alias path: the deterministic ELF fixture enters at virtual RIP `0x400100` through a validated RAM-backed alias PTE and terminates at virtual RIP `0x400124`.

The virtual-MMIO composition fixture keeps instruction fetch identity-mapped at RIP `0x10000`, materializes VA `0x500000` in RBX, writes `W` and reads `R` through that virtual address, and relies on the device PTE to translate both accesses to unbacked GPA `0x10000000`. KVM therefore reports two typed MMIO exits at the translated physical address. After userspace supplies the read byte and re-enters KVM, the guest emits `R64M` through four port exits and halts at RIP `0x1001e`. This proves translation and MMIO execution compose without introducing a general page-table manager or arbitrary virtual-MMIO API.

The direct-interrupt fixture requests KVM's interrupt window before injection. The main guest executes `STI` followed by one `NOP`; KVM must return `KVM_EXIT_IRQ_WINDOW_OPEN` after the architectural STI shadow with RIP `0x10002`, `ready_for_interrupt_injection=1`, and IF set. Userspace then injects vector `0x40`. The installed interrupt gate transfers control to handler `0x11000`; the handler emits `I`, executes `IRETQ`, and the resumed main path emits `M` before HLT at RIP `0x10007`. The resulting ready-window plus `IM` proof demonstrates synchronized vector injection, delivery, handler execution, interrupt return, and resumed guest execution; terminal RFLAGS retains architectural bit 1 and IF.

## vCPU execution

The legacy real-mode fixtures start from KVM's new-vCPU reset state, normalize CS/DS/ES/FS/GS/SS base/selectors to zero, clear CR0 protected-mode/paging enable bits, then set a zeroed `kvm_regs` with RIP and architectural RFLAGS bit 1. Their CS=0 entry remains deliberately limited to `0xffff`. The real-mode MMIO fixture uses this same initialization contract at RIP `0x100`.

The long-mode, ELF64, virtual-MMIO, and direct-interrupt fixtures follow the validated long-mode state contract above. They do not transit through a guest-side real-to-protected-to-long-mode boot stub; userspace establishes the architectural long-mode state through KVM sregs/regs before the first `KVM_RUN`. Only the interrupt fixture additionally installs guest-memory descriptor tables, requests and validates an injection-ready KVM interrupt window, and then injects one external vector.

`Vcpu::capture_register_snapshot` performs one existing `KVM_GET_REGS` and copies all 18 x86 general-register fields into an owned `VcpuRegisterSnapshot`. Pure comparison, read-only verification, snapshot-bound restore, and restore-and-verify remain unchanged by the long-mode bootstrap.

Special-register capture likewise owns semantic x86 segment, descriptor-table, control-register, EFER, APIC-base, and interrupt-bitmap state without exposing KVM padding. Pure comparison and read-only verification remain separate from restore. `VcpuStateSnapshot` composes general-register, special-register, and policy-bound MSR snapshots with explicitly bounded non-transactional semantics. None of these values is a whole-VM, guest-memory, device-state, checkpoint, migration, atomic/quiesced snapshot, or rollback primitive.

`Vcpu::run_once` retries an interrupted host syscall, performs one completed `KVM_RUN`, reads only tested x86 `kvm_run` prefixes, and returns a typed `VcpuExit`. HLT, port I/O, MMIO, legacy shutdown, KVM unknown, exception, fail-entry, internal-error, and system-event exits are classified explicitly; other unsupported reasons retain their exact raw reason.

For `KVM_EXIT_IO`, `Vcpu::port_io_exit` validates direction, `data_offset`, checked `size * count`, the complete mapped range, and owned OUT copying. `Vcpu::write_port_io_input` independently validates IN direction and exact response length before writing owned bytes to the pending KVM data range. No pointer into `kvm_run` leaves the vCPU layer.

For `KVM_EXIT_MMIO`, `Vcpu::mmio_exit` validates the current reason, accepts only read/write direction, requires a non-zero length no greater than KVM's fixed 8-byte payload capacity, copies only declared write bytes, and publishes an owned `MmioExit`. Read exits expose no stale data bytes. `Vcpu::write_mmio_read_response` revalidates the pending MMIO exit, requires read direction and exact response length, then writes only the validated prefix of KVM's fixed MMIO data array. The long-mode path uses this exact same physical-exit boundary after guest page-table translation; no virtual-address metadata is guessed from the exit payload.

Purpose-built KVM-unknown, exception, fail-entry, internal-error, and system-event decoders validate the current reason before inspecting their union member, copy required fields into owned Rust state, bound every variable-length count before slicing, and keep higher-level dispatch free of raw shared-memory pointers.

## Bounded execution loop

`execution::run_vcpu_until_stopped` remains the source-compatible reusable run-loop boundary for callers that only service port I/O. It constructs an empty MMIO bus and delegates to `run_vcpu_until_stopped_with_mmio`, which is the common implementation used by both MMIO fixtures. Before each `KVM_RUN` the common loop checks an explicit completed-exit budget. A successful `KVM_RUN` consumes exactly one budget unit; host-side failures that do not produce a completed VM exit consume none.

Each completed exit is recorded exactly once in an ordered raw reason trace before dispatch. Serviceable port I/O and MMIO are recorded as owned typed exits and execution continues while budget remains. A terminal HLT or legacy shutdown returns `VmExecutionResult`, which contains the terminal `VmExitReport`, serviced typed I/O/MMIO exits, the exact completed-exit count, and the complete ordered raw reason trace.

A zero budget fails before any guest run. Budget exhaustion is structured failure, not guest termination. If the final permitted exit was serviceable I/O or MMIO, userspace may have prepared the service response but the VMM does not claim KVM completed the pending operation without another permitted `KVM_RUN`.

The interrupt-window `KVM_RUN` is an explicit pre-injection synchronization step outside the reusable terminal/device loop, so its `KVM_EXIT_IRQ_WINDOW_OPEN` is validated separately rather than counted in `VmExecutionResult`. After injection, the direct-interrupt fixture uses loop budget 3 for exactly handler proof I/O, resumed-main proof I/O, and HLT; the `KVM_INTERRUPT` ioctl itself is not a VM exit. The HLT and CPUID fixtures use budget 1. The real-mode debug-port fixtures use budget 2. The flat long-mode and ELF64 proofs use budget 5 and succeed only with the exact sequence of four serviced I/O exits followed by terminal HLT. Both MMIO proof fixtures use budget 7 and prove two serviced MMIO exits followed by four debug-port I/O exits and HLT; extra exits consume the budget and prevent milestone success.

## VM-exit dispatch

`vmexit::dispatch_vcpu_exit` is the single policy boundary for one completed vCPU exit. HLT and legacy shutdown become `VmExitReport`; I/O is serviced through `PortIoBus`; MMIO is serviced through `MmioBus`; KVM-unknown, exception, fail-entry, internal-error, and system-event exits become their existing structured diagnostics; other unsupported raw reasons remain generic unhandled errors.

The dispatcher deliberately does not snapshot registers for an in-flight KVM I/O or MMIO exit because KVM defines the operation as pending until userspace re-enters `KVM_RUN`. Register state used as a completed-operation diagnostic is therefore taken on the later terminal exit.

The deterministic real-mode output fixture reaches HLT at RIP `0x1005`; the input fixture reaches `0x1006`; the CPUID fixture reaches `0x101c`. The real-mode MMIO fixture writes `W`, receives `R`, emits `RMIO`, and reaches HLT at RIP `0x117`. The flat x86-64 proof emits `LM64` across four I/O exits and reaches HLT at RIP `0x10024`; the non-identity ELF64 proof emits the same bytes after production ELF materialization and reaches HLT at virtual RIP `0x400124`; the long-mode virtual-MMIO proof translates VA `0x500000` to device GPA `0x10000000`, writes `W`, receives `R`, emits `R64M`, and reaches HLT at RIP `0x1001e`; the direct-interrupt proof first observes the ready IRQ window at RIP `0x10002`, then emits `I` from its vector-`0x40` handler, returns with `IRETQ`, emits `M` from resumed main code, and reaches HLT at RIP `0x10007`.

## Port-I/O bus and debug device

`PortIoBus` remains intentionally minimal. It contains only the exact debug-port device at port `0xe9`; it is not a dynamic device registry or port-range resolver. The device accepts only byte-wide, single-count accesses. OUT appends one copied byte to the output buffer; IN returns one configured owned byte. Unknown ports, wide/multi-count operations, payload mismatches, and response-length mismatches are explicit errors.

The long-mode, ELF64, real-mode MMIO, virtual-MMIO, and direct-interrupt proof fixtures reuse this exact existing port path for host-visible proof output.

## MMIO bus and byte device

`MmioBus` is deliberately a second minimal device-policy boundary rather than an extension of port I/O semantics. `with_byte_device` preserves the original exact GPA `0x2000` fixture contract; `with_byte_device_at` places the same one-byte device policy at one explicit caller-supplied GPA. The bus still owns at most one byte device and does not resolve ranges or multiple devices. A write records the copied byte in owned device state. A read returns one configured owned byte. Unknown addresses, non-byte widths, malformed write payloads, and invalid response lengths fail explicitly.

The real-mode MMIO fixture registers only `0..0x1000` as RAM, so its access to default GPA `0x2000` reaches userspace through `KVM_EXIT_MMIO`. The long-mode virtual-MMIO fixture instead configures the same exact byte-device policy at GPA `0x10000000`, which remains outside its 2 MiB RAM slot and is reached through the fixed VA `0x500000` PTE. Neither address is a global allocator reservation. There is no MMIO range registry, multiple-device resolution, register bank, PCI bus, eventfd acceleration, DMA model, or caller-defined virtual-MMIO mapping.

## Ownership and lifetime

`KvmBackend` owns `/dev/kvm`, validated capability and CPUID/MSR discovery snapshots, and the configured guest CPU policy. `Vm` owns the VM descriptor, guest policy, optional internal-error capability observation, and registered guest RAM. `Vcpu` owns the vCPU descriptor and `KvmRunMapping`. CPU/MSR snapshots, diagnostics, `PortIoExit`, `MmioExit`, `VmExecutionResult`, and fixture result types own copied Rust data rather than pointers into KVM shared memory or guest RAM.

`LongModeBootLayout` owns a validated architectural entry address, low-RAM/stack layout, and owned bounded RAM-backed alias page mappings; it contains no host pointer, mapping borrow, vCPU descriptor, or raw KVM state. `LongModeMmioBootLayout` owns one validated base boot layout and fixed device-mapping semantics; `LongModeInterruptLayout` owns one validated base boot layout plus exact vector, handler, and descriptor-table bases/limits. Neither composition layout owns a host pointer or KVM descriptor. Page-table and descriptor-table installation mutate guest RAM only through checked `GuestMemory` writes. `LongModeGuestResult` and direct-interrupt result values own copied proof/output/terminal state; the direct-interrupt result additionally owns copied interrupt-window RIP/RFLAGS observations rather than a pointer into `kvm_run`. `Elf64GuestImage` borrows immutable input bytes while owning validated virtual/physical load-segment metadata and its derived RAM-backed alias mapping plan; `Elf64GuestResult` owns copied I/O exits, proof bytes, and its terminal report. `MmioBus` owns its exact configured address, configured read byte, and captured write bytes; the real-mode and long-mode MMIO fixture results own copied MMIO exits, port exits, device writes, proof bytes, and terminal report.

Rust ownership is used for normal cleanup; explicit KVM slot removal protects the guest-RAM lifetime boundary when independent vCPU descriptors exist.

## Error boundary

Errors remain categorized as host environment, KVM capability, configuration, guest memory, guest image, VM exit, port I/O, and MMIO errors. Pure guest-MSR policy/value/snapshot validation keeps its dedicated typed errors.

Long-mode RAM-backed layout validation is a pure configuration boundary represented by `LongModeConfigurationError`; invalid RAM base/size, identity/alias entry mapping, page-table overlap, stack mapping, malformed alias pages, out-of-window virtual pages, out-of-RAM or bootstrap-overlapping physical pages, or duplicate virtual mappings are rejected before page-table installation or KVM long-mode state configuration. `LongModeMmioConfigurationError` wraps those base boot failures and independently rejects a slot-0 RAM extent that would cover fixed device GPA `0x10000000`; it does not convert that GPA into a RAM mapping. `LongModeInterruptConfigurationError` likewise wraps base boot failures and adds typed rejection for exception-reserved vectors, entry/handler descriptor-table collisions, handlers outside the low identity map, and a bounded interrupt frame overlapping `0x1000..0x7000`. Page-table/GDT/IDT writes still use the existing `GuestMemory` error boundary. `Vcpu::initialize_long_mode`, `initialize_long_mode_interrupts`, `wait_for_interrupt_window`, and `inject_interrupt` preserve named failures for `KVM_GET_SREGS`, `KVM_SET_SREGS`, `KVM_SET_REGS`, the interrupt-window `KVM_RUN` handshake/validation, `KVM_GET_REGS`, and `KVM_INTERRUPT`. Runtime proof failure remains an execution/VM-exit failure rather than being converted into successful milestone completion.

ELF64 format and layout validation uses `Elf64Error` before guest memory is mutated: malformed headers/program-header tables, unsupported class/endianness/type/machine/version, invalid file/virtual/physical ranges, invalid alignment, invalid identity/alias placement, bootstrap overlap, segment overlap, conflicting mapping plans, and an invalid entry are rejected as typed loader errors. Once validated, materialization reuses the existing checked `GuestMemory` error boundary.

MMIO UAPI-validation failures remain `VmExitError` values because they describe an invalid pending KVM exit payload or response mutation request. Device-policy failures are `MmioError` values and preserve address/direction/length metadata. Neither category is swallowed or translated into a successful execution result.

Future irqchip/APIC, whole-VM/device-snapshot, and stronger invariant categories will be added only when those responsibilities exist.

## Deliberate non-abstractions

There is no generic hypervisor backend trait yet. KVM is the only implementation, and an abstraction would not have a second consumer.

There is no configurable or migration-stable CPU model yet. Current CPUID and immutable-MSR model-candidate boundaries remain host-derived analysis contracts rather than cross-host migration guarantees.

The implemented state lifecycle is deliberately vCPU-CPU-state scoped and non-transactional across multi-component restore. There is no automatic mismatch repair, rollback, multi-vCPU restore orchestration, guest-memory/device snapshot, checkpoint decoder, or migration protocol.

The long-mode mapping is deliberately **not** a generic virtual-memory subsystem. It preserves one fixed 2 MiB identity map and one fixed 2 MiB alias virtual window implemented by a single fixed 4 KiB page-table page. RAM-backed `LongModePageMapping` remains restricted to validated low-RAM physical pages. `LongModeMmioBootLayout` adds exactly one fixed device PTE from VA `0x500000` to unbacked GPA `0x10000000`; it does not add an allocator, arbitrary VA→GPA API, caller-defined device mapping, caller-defined window, page-permission/NX policy surface, or guest-controlled page-table construction path.

The direct interrupt path is deliberately **not** an interrupt-controller subsystem. It installs one fixed VMM-owned GDT/IDT layout, uses one bounded KVM interrupt-window handshake, and directly injects one vector through `KVM_INTERRUPT` while no in-kernel irqchip exists. There is no PIC/local-APIC/IOAPIC model, IRQ pin or GSI routing, MSI/MSI-X, timer, priority arbitration, reusable interrupt-window scheduler, multiple pending-vector queue, cross-vCPU routing, or device-generated interrupt wiring. Consequently the no-LAPIC CPUID masks remain in force.

The ELF64 loader is deliberately **not** a general ELF runtime. It supports only bounded x86-64 little-endian `ET_EXEC` `PT_LOAD` materialization into low RAM plus identity placement or the fixed alias window. It has no relocations, load bias, `ET_DYN`/PIE, dynamic linker/interpreter, symbol model, section-driven loading, or general VA layout policy.

Typed KVM-unknown, exception, fail-entry, internal-error, and system-event diagnostics remain diagnostics. They do not imply retry, recovery, exception injection, instruction emulation, placement, or lifecycle policy.

There is no multi-region memory map yet. `GuestMemoryRegion::overlaps` exists to make range semantics explicit and tested, but the VM intentionally supports only slot 0. The MMIO proofs demonstrate that an address outside that one slot can be serviced as userspace MMIO directly or after guest page-table translation; they do not add another RAM slot or hole-management API.

The port bus is not a trait-object registry yet. One exact bidirectional device is enough to prove checked OUT/IN behavior and provide proof output for long-mode, ELF64, MMIO, and interrupt fixtures without introducing registration/range-resolution machinery prematurely.

The MMIO bus is also not a generic registry. One exact byte-device policy, placeable at one explicit address, proves checked write capture, read response, pending-operation completion, and long-mode translation composition. Multiple simultaneous devices, ranges, register banks, or accelerated eventfds require a separately selected executable milestone.

The execution loop is not a scheduler. It owns no vCPU, thread, timer, or interrupt-controller state; it only bounds repeated execution of one already-created vCPU. The direct-interrupt fixture performs its one interrupt-window handshake and vector injection explicitly before entering this reusable loop.

## Next architectural milestone

`ROADMAP.md` is the authoritative live source for milestone selection. Long-mode virtual-MMIO composition is integrated. The current selected promotion is one bounded direct long-mode external-vector delivery path that proves GDT/IDT installation, KVM interrupt-window synchronization, `KVM_INTERRUPT`, handler execution, `IRETQ`, resumed execution, and terminal HLT without claiming an irqchip/APIC model. Once this slice is integrated and exact post-merge `main` CI is green, perform another architecture/integration audit. The next interrupt frontier should introduce real controller/routing/device-delivery semantics only as one coherent executable slice; do not farm additional fixed direct-vector variants.

## Internal-error emulation-failure metadata

`VcpuInternalError` exposes read-only interpretation for the stable x86 `KVM_INTERNAL_ERROR_EMULATION` metadata already copied into its owned optional-data words. These accessors are pure reads over already-owned diagnostic state, form no additional `kvm_run` view, perform no ioctl, and introduce no emulation recovery or execution policy.

## KVM exception diagnostics

`KVM_EXIT_EXCEPTION` raw reason `1` is a distinct typed `VcpuExit::Exception` path. `Vcpu::exception_exit()` validates the reason before reading the fixed x86 union member, copies exception vector/error code into owned `VcpuException`, and dispatch returns the existing structured diagnostic without a secondary register ioctl. Exception metadata remains opaque and grants no injection, reinjection, emulation, retry, recovery, or lifecycle authority.
