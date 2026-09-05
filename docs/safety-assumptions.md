# Safety assumptions

## Trust model

The Linux KVM kernel interface and explicitly supplied host process configuration are trusted. Guest-originated addresses, lengths, CPU-visible values, port-I/O requests, MMIO requests, exit metadata, executable-image metadata, and device inputs are not trusted merely because KVM or a caller produced or consumed them. Userspace validates every value that becomes a Rust slice length, host-memory offset, guest-memory range, state-policy input, shared `kvm_run` write, page-table mapping decision, or higher-level execution decision.

The repository-owned HLT, debug-port, CPUID, real-mode MMIO, x86-64 long-mode, long-mode virtual-MMIO, and ELF64 proof fixtures are reviewed deterministic test inputs. `FlatGuestImage` still validates non-empty bytes, load-range arithmetic, and entry containment. `Elf64GuestImage` treats the supplied byte slice and all ELF header/program-header metadata as untrusted even when the deterministic fixture is the caller.

The long-mode path does not accept arbitrary page tables or arbitrary special-register state from the guest or a caller. `LongModeBootLayout` owns a validated bootstrap description. Identity-only layouts use the fixed low 2 MiB mapping; mapped layouts may additionally contain validated RAM-backed `LongModePageMapping` entries restricted to the fixed alias window and low-RAM backing described below. `LongModeMmioBootLayout` is a separate bounded composition that installs exactly one fixed device PTE without relaxing those RAM-backed mapping rules. `Vcpu::initialize_long_mode` materializes only the project-defined CPU state.

The MMIO path is similarly bounded. Device policy does not receive a raw pointer into `kvm_run`; it receives an owned `MmioExit` containing validated direction/length/address metadata and copied write bytes. The current `MmioBus` owns at most one exact byte-wide device at one configured guest-physical address and rejects unsupported addresses or widths rather than guessing device semantics.

## Unsafe boundary

Raw unsafe host interaction remains limited to Linux KVM ioctls, ownership conversion of successful KVM-created file descriptors, and `mmap`/`munmap` used for guest RAM and `kvm_run`. KVM UAPI structures are represented by tested fixed-layout or bounded `repr(C)` Rust structures.

Kernel-returned variable-length metadata is never used as a Rust slice length before validation. This includes supported/read-back CPUID counts, general/feature MSR-index counts, system and vCPU MSR completion counts/index metadata, capability-enabled internal-error `ndata`, system-event `ndata`, port-I/O ranges, and MMIO lengths.

Pointers into `kvm_run`, temporary KVM request buffers, and host pointers for guest RAM do not escape into VM-exit policy, device policy, execution results, snapshot values, long-mode layout values, or ELF64 image metadata.

`KVM_EXIT_MMIO` uses a tested fixed x86 union view. The vCPU layer validates the current exit reason before forming that view, accepts only read/write direction values, requires `len` in `1..=8`, copies only declared write bytes into owned Rust state, and exposes no stale `data[]` contents on read exits. MMIO read responses are written back only after revalidating read direction and requiring response length to equal the declared access length exactly. KVM reports the guest-physical address after guest page-table translation; the userspace MMIO decoder does not infer or reconstruct the originating guest virtual address.

## Guest memory and long-mode bootstrap safety

Guest physical addresses use `GuestPhysAddr`; they are never cast directly to host pointers. `GuestMemory` owns a private anonymous host mapping and validates guest address plus length before any host pointer arithmetic or copy. KVM slot 0 registration occurs only after region validation and overlap rejection against the high x86 KVM-reserved identity-map/TSS range `0xfeff_c000..0xff00_0000`.

The dedicated real-mode MMIO proof registers only `0x0000..0x1000` as RAM and deliberately accesses GPA `0x2000`, which is outside that fixture's registered memory. That fixture-specific choice is what causes KVM to exit through the userspace MMIO path. It does not reserve `0x2000` globally: the long-mode bootstrap uses the same GPA as its PDPT page, and other fresh fixtures may also use it as ordinary RAM. Fixtures are mutually exclusive VM layouts.

The long-mode virtual-MMIO proof instead uses the established 2 MiB slot-0 RAM layout and fixed device GPA `0x10000000`. `LongModeMmioBootLayout::new` first validates the base long-mode layout and then rejects any memory region whose exclusive end extends beyond that fixed GPA. Because the base layout already requires RAM to begin at zero, this proves the device page remains outside registered slot-0 RAM rather than silently becoming normal memory.

The x86-64 bootstrap requires one RAM region beginning at GPA 0 with at least 2 MiB. The identity-only constructor rejects a non-zero RAM base, RAM smaller than `0x20_0000`, an entry outside the low identity map or inside bootstrap page-table pages, a zero/out-of-map stack pointer, or a stack top overlapping those pages. `LongModeBootLayout::with_page_mappings` retains the same RAM/stack rules and additionally validates each 4 KiB RAM-backed alias mapping before any page-table write:

- the virtual page must be 4 KiB aligned and inside `0x40_0000..0x60_0000`;
- the physical backing page must be 4 KiB aligned and entirely inside the low 2 MiB RAM;
- physical backing may not overlap bootstrap page tables `0x1000..0x5000`;
- the same alias virtual page may not appear twice;
- an entry in the alias window is accepted only if its containing page exists in the validated mapping set;
- an entry outside both the low identity map and the fixed alias window is rejected.

`LongModeMmioBootLayout` does not construct a `LongModePageMapping`. After installing the validated identity-only base tables, it links PD[2] to the existing alias PT and writes exactly PTE index 256, mapping virtual page `0x500000` to fixed unbacked GPA page `0x10000000` with the same present/writable flags. The virtual page and device GPA are fixed 4 KiB-aligned constants rather than caller-selected mapping input. Its executable entry remains identity-mapped at `0x10000`, so the device PTE is used only for data access.

The flat deterministic long-mode and long-mode virtual-MMIO proofs use identity entry `0x10000`; the ELF64 proof uses virtual entry `0x400100` backed by low-RAM GPA `0x10100`; all use stack pointer `0x1ff000` inside the preserved identity map.

Page-table construction performs no raw pointer arithmetic. Four full 4 KiB pages at GPA `0x1000`, `0x2000`, `0x3000`, and `0x4000` are zeroed through `GuestMemory::write`. PML4[0] is `0x2003`, PDPT[0] is `0x3003`, and PD[0] is `0x83`, preserving exactly one present/writable 2 MiB identity mapping for VA/GPA `0..0x20_0000`. Identity-only layouts leave the alias PDE absent. When bounded RAM-backed alias mappings are present, PD[2] points to GPA `0x4000` with present/writable flags, and only validated PTE slots are populated with validated low-RAM physical pages plus the same flags; all unused alias PTEs stay zero. The virtual-MMIO composition reuses that same checked page-table memory and mutates only the fixed PD[2]/PTE[256] entries after its separate unbacked-device validation.

This mapping layer is intentionally bounded rather than a general guest virtual-address translation facility. There is no arbitrary virtual window, dynamic page-table allocation, caller-defined hierarchy, per-page executable/write policy, NX policy, caller-defined or arbitrary virtual-MMIO mapping, guest-supplied page-table parser, or page-fault recovery policy.

## ELF64 loader safety

`Elf64GuestImage::parse` accepts only ELF64 little-endian x86-64 `ET_EXEC`. It validates the fixed header size and program-header entry size, converts and bounds the complete program-header table before traversing it, and never trusts a file offset or count as a Rust slice boundary without checked conversion and arithmetic.

For each `PT_LOAD`, validation requires non-zero memory size, `p_filesz <= p_memsz`, a file-backed range entirely inside the supplied bytes, independently checked virtual and physical extents, and host-size conversions that cannot overflow. Segment alignment must be 0, 1, or a power of two; aligned segments must satisfy ELF offset/virtual-address congruence. Physical backing must stay wholly inside the low 2 MiB RAM and outside bootstrap page tables `0x1000..0x5000`.

A segment whose virtual range lies inside the low identity window must keep `p_vaddr == p_paddr`. A non-identity segment is accepted only when its complete virtual range lies inside `0x40_0000..0x60_0000` and its virtual/physical addresses have the same 4 KiB page offset. Virtual load ranges may not overlap one another, physical backing ranges may not overlap one another, and the generated alias mapping plan rejects conflicting virtual-page mappings before it reaches `LongModeBootLayout`.

An ELF entry is accepted only inside the file-backed portion of an executable `PT_LOAD`; an entry that exists only in a zero-filled BSS tail is rejected. Materialization occurs only after the whole image has been validated. File-backed bytes are copied through checked `GuestMemory::write` to the validated physical backing, and each physical BSS tail is explicitly zeroed before KVM memory registration. Virtual addresses are never used as host offsets. The deterministic fixture intentionally contains a non-empty BSS tail so this behavior is regression-tested rather than merely documented.

This loader does not perform relocation, load-bias selection, `ET_DYN`/PIE loading, dynamic linking/interpreter handoff, symbol resolution, section-driven loading, arbitrary virtual-window selection, or dynamic page-table construction. Absence of those features is part of the safety boundary, not an implicit best-effort behavior.

## Long-mode vCPU state safety

`Vcpu::initialize_long_mode` first reads the vCPU's current KVM special-register state and then mutates only the fields required by the validated bootstrap contract. It ORs the required `CR0.PE|CR0.PG`, `CR4.PAE`, and `EFER.LME|EFER.LMA` bits so unrelated inherited bits are not silently cleared, and sets `CR3` exactly to the validated PML4 GPA `0x1000`.

The segment state is not supplied by guest bytes. CS is a fixed present ring-0 64-bit code segment with selector `0x8`, base 0, limit `0xffff_ffff`, `L=1`, and `DB=0`. DS/ES/FS/GS/SS use the fixed present ring-0 data-segment contract with selector `0x10`, base 0, and the same limit. RIP is the validated architectural entry address and may therefore be the identity entry or a mapped RAM-backed alias virtual address; the virtual-MMIO fixture deliberately retains identity RIP. RSP remains the validated low identity-mapped stack pointer. RFLAGS is initialized with architectural bit 1 set and all remaining general-register fields begin from zero.

Failure of `KVM_GET_SREGS`, `KVM_SET_SREGS`, or `KVM_SET_REGS` is a named vCPU-operation error. The implementation does not retry a partially applied state sequence or claim transactional rollback. In deterministic proof paths, failure prevents a successful proof result.

## Deterministic executable proofs

The reviewed 36-byte flat x86-64 fixture is loaded at GPA/VA `0x10000`. It uses 64-bit-width instruction encodings, emits bytes `L`, `M`, `6`, and `4` through four byte-wide single-count OUT operations on the existing debug port `0xe9`, then executes `HLT`.

The ELF64 proof wraps the same architectural proof code inside the production `ET_EXEC` loader path but deliberately executes through a non-identity RAM-backed mapping. Its executable segment has virtual base `0x400000`, physical backing base `0x10000`, virtual entry `0x400100`, and a larger memory size than file size so the loader must zero physical BSS before execution. The validated page-table plan maps virtual page `0x400000` to physical page `0x10000`.

Each of those two long-mode proofs uses an execution budget of exactly five completed exits. Success requires four serviced I/O exits in order followed by the typed terminal HLT report. The host-owned proof buffer must equal `LM64`; terminal RIP is `0x10024` for the flat identity fixture and `0x400124` for the non-identity ELF64 fixture. Budget exhaustion, another exit, malformed port-I/O metadata, an unsupported port operation, invalid executable metadata, or KVM entry/execution failure is not converted into milestone success.

The reviewed real-mode MMIO fixture is a separate 23-byte program at RIP `0x100` inside a dedicated 4 KiB RAM region. It performs one byte write of `W` to unbacked GPA `0x2000`, then one byte read from the same device. Userspace returns `R`; after KVM completes that pending read on re-entry, the guest emits `R`, `M`, `I`, `O` through the existing debug port and halts at RIP `0x117`. The proof therefore requires seven completed exits in exact semantic order: MMIO write, MMIO read, four port-I/O outputs, then HLT. Device-captured writes must equal `W` and host-captured debug output must equal `RMIO`.

The reviewed long-mode virtual-MMIO fixture is a 30-byte 64-bit program loaded identity-mapped at GPA/VA `0x10000`. It materializes virtual address `0x500000` in RBX, performs a one-byte write of `W` and one-byte read through that VA, and relies on the fixed device PTE to translate both operations to unbacked GPA `0x10000000`. The device returns `R`; after userspace writes that read response and KVM completes the pending access on re-entry, the guest emits `R`, `6`, `4`, `M` through port `0xe9` and halts at RIP `0x1001e`. This proof also uses exactly seven completed exits: translated MMIO write, translated MMIO read, four port-I/O outputs, then HLT. The typed MMIO exits must report physical address `0x10000000`, captured device writes must equal `W`, and host-captured proof output must equal `R64M`.

KVM-aware Rust regressions follow the repository's general environment-sensitive convention. In addition, CI contains strict gates that directly run `run-long-mode`, `run-elf64`, `run-mmio`, and `run-long-mode-mmio` with usable `/dev/kvm`, check the exact proof bytes, check each exact terminal HLT RIP, and require architectural RFLAGS bit 1. Those gates fail if KVM is unavailable and provide evidence that each candidate actually executed the guest path rather than only validating pure construction.

## Port-I/O, MMIO, and execution-loop safety

For `KVM_EXIT_IO`, `data_offset` is an offset into the owned `kvm_run` mapping, never a trusted pointer. The vCPU layer checks integer conversion, checked `size * count`, checked range-end addition, and the final range against the mapping before any pointer arithmetic. OUT bytes are copied into owned Rust memory before device policy sees them.

For `KVM_EXIT_IO_IN`, device policy returns owned response bytes. The vCPU layer revalidates the current I/O metadata, requires IN direction, recomputes the checked range, and requires exact response length before copying bytes back into `kvm_run`.

For `KVM_EXIT_MMIO`, device policy sees only the owned typed physical access. Unknown addresses and non-byte-wide accesses are explicit `MmioError` failures in the current exact byte-device contract. Write payload size must match the declared access exactly. A read response is copied into KVM's fixed MMIO `data[]` array only for a validated read exit and exact length. `MmioBus::with_byte_device` retains default fixture GPA `0x2000`; `with_byte_device_at` changes only the one exact configured device address and does not create a range registry or multiple-device routing policy.

KVM defines serviced I/O/MMIO completion as pending until userspace re-enters `KVM_RUN`. The execution loop therefore does not claim completed post-device state until a later completed exit. The explicit exit budget is checked before each run; only a successful completed KVM exit consumes one unit. Budget exhaustion remains a structured error, not a terminal guest report.

The long-mode virtual-MMIO path does not alter MMIO payload semantics. Guest translation occurs before KVM reports the exit, so the same checked decoder sees GPA `0x10000000`; the VMM does not trust or infer the originating VA from that payload. The correspondence to fixed VA `0x500000` comes from the separately validated page-table installation plus deterministic real-KVM execution proof.

## CPUID and MSR safety boundaries

Supported/read-back CPUID uses bounded `KvmCpuid2<N>` storage. Kernel counts are validated before slicing; KVM padding is not retained in owned typed state. Guest CPUID policy is derived from owned host support and applied/read back exactly before a vCPU is published. The current policy conservatively removes LAPIC-dependent x2APIC, TSC-deadline, and PV-unhalt exposure while no LAPIC/IRQ-chip model exists.

MSR index and value discovery uses bounded `repr(C)` request objects. Returned processed counts and entry indices are validated exactly before owned typed state is published. Caller-selected guest MSR authorization is validated against the general host index snapshot; caller-selected values are validated against that policy; full snapshots additionally require complete coverage and exact policy order.

KVM MSR writes are not treated as transactional. A short processed count is reported as a partial write because the successful prefix may already have changed vCPU state. Restore and restore-and-verify do not retry or roll back that prefix. None of the CPUID/MSR model or snapshot comparison types constitutes a migration-safety decision.

## VM-exit diagnostic safety

Typed KVM-unknown, exception, fail-entry, internal-error, and system-event paths validate the current raw exit reason before interpreting the corresponding tested `kvm_run` union view. Required scalars and bounded payloads are copied into owned Rust state before higher-level policy sees them.

`KVM_EXIT_UNKNOWN` hardware reason, exception vector/error code, fail-entry hardware reason/CPU, internal-error suberror/data, and system-event type/data are diagnostic metadata. They do not grant authority for retry, recovery, exception injection/reinjection, instruction emulation, CPU placement, replacement execution, or lifecycle mutation.

Optional internal-error data is formed only when the propagated `KVM_CAP_INTERNAL_ERROR_DATA` observation is positive. `ndata` must be `<= 16` before slicing. Typed internal-error suberror classification is a pure view over the already copied raw scalar and preserves unknown values losslessly. Emulation instruction-byte helpers operate only on already-owned optional words and reject an instruction size above the fixed 15-byte overlay before slicing.

`KVM_EXIT_EXCEPTION` uses only the fixed two-`u32` payload at the x86 union offset; the resulting owned vector/error-code diagnostic does not trigger a secondary register ioctl that could obscure the completed exit.

## CPU-state snapshot safety

General-register, special-register, policy-bound MSR, and composite vCPU snapshots own copied typed CPU state. Pure comparison and read-only verification do not invoke restore. Restore operations use existing validated KVM setters and are explicitly non-transactional across multiple component writes. These values are not whole-VM, guest-memory, device-state, checkpoint, migration, rollback, or atomic/quiesced snapshot primitives.

The long-mode bootstrap uses the same KVM register/special-register structures but does not turn those state snapshot APIs into a boot-state parser or migration format. The MMIO device's captured write bytes are execution-fixture evidence, not snapshot or migration state.

## VM and memory lifetime

Successful `KVM_CREATE_VM` and `KVM_CREATE_VCPU` results are immediately wrapped in `OwnedFd`. `Vm` owns registered guest RAM after successful slot registration. Before releasing RAM it attempts to unregister slot 0 with a zero-sized memory-region update. If unregister fails while an independent vCPU descriptor could keep the kernel VM alive, the userspace mapping is intentionally leaked rather than unmapped underneath a potentially live KVM slot.

## Not yet present

The repository now has one fixed low 2 MiB x86-64 identity mapping, one bounded 2 MiB alias virtual window, RAM-backed ELF aliases restricted to validated low-RAM physical pages, one fixed long-mode device PTE mapping VA `0x500000` to deliberately unbacked GPA `0x10000000`, one bounded ELF64 `ET_EXEC` loader/execution path, and one exact byte-wide userspace MMIO device policy that can be placed at one explicit GPA. It still has **no general virtual-memory subsystem** or arbitrary guest address-space construction. It also has no ELF relocations, `ET_DYN`/PIE or dynamic-linker path, dynamic page-table allocator, page-permission/NX policy, Linux boot protocol, MMIO range registry, multiple simultaneous MMIO devices/register banks, arbitrary or caller-defined virtual-MMIO mapping, PCI, APIC/interrupt controller, interrupt injection framework, virtio, eventfd/ioeventfd/irqfd acceleration, DMA/IOMMU model, SMP, dynamic device registration, disk backend, whole-VM/guest-memory/device snapshot orchestration, migration protocol, resumable execution, scheduler, exception recovery/injection policy, KVM-unknown recovery policy, fail-entry retry/placement policy, internal-error recovery policy, or system-event lifecycle policy.

Those responsibilities require separately selected milestones. The bounded long-mode virtual-MMIO composition milestone does not authorize them implicitly.
