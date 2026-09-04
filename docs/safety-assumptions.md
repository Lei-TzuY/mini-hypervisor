# Safety assumptions

## Trust model

The Linux KVM kernel interface and explicitly supplied host process configuration are trusted. Guest-originated addresses, lengths, CPU-visible values, port-I/O requests, exit metadata, executable-image metadata, and future device/MMIO inputs are not trusted merely because KVM or a caller produced or consumed them. Userspace validates every value that becomes a Rust slice length, host-memory offset, guest-memory range, state-policy input, or higher-level execution decision.

The repository-owned HLT, debug-port, CPUID, x86-64 long-mode, and ELF64 proof fixtures are reviewed deterministic test inputs. `FlatGuestImage` still validates non-empty bytes, load-range arithmetic, and entry containment. `Elf64GuestImage` treats the supplied byte slice and all ELF header/program-header metadata as untrusted even when the deterministic fixture is the caller.

The long-mode milestone does not accept arbitrary page tables or arbitrary special-register state from the guest or a caller. `LongModeBootLayout` is a validated fixed bootstrap description, and `Vcpu::initialize_long_mode` materializes only the project-defined state described below.

## Unsafe boundary

Raw unsafe host interaction remains limited to Linux KVM ioctls, ownership conversion of successful KVM-created file descriptors, and `mmap`/`munmap` used for guest RAM and `kvm_run`. KVM UAPI structures are represented by tested fixed-layout or bounded `repr(C)` Rust structures.

Kernel-returned variable-length metadata is never used as a Rust slice length before validation. This includes supported/read-back CPUID counts, general/feature MSR-index counts, system and vCPU MSR completion counts/index metadata, capability-enabled internal-error `ndata`, and system-event `ndata`.

Pointers into `kvm_run`, temporary KVM request buffers, and host pointers for guest RAM do not escape into VM-exit policy, device policy, execution results, snapshot values, long-mode layout values, or ELF64 image metadata.

## Guest memory and long-mode bootstrap safety

Guest physical addresses use `GuestPhysAddr`; they are never cast directly to host pointers. `GuestMemory` owns a private anonymous host mapping and validates guest address plus length before any host pointer arithmetic or copy. KVM slot 0 registration occurs only after region validation and overlap rejection against the high x86 KVM-reserved identity-map/TSS range `0xfeff_c000..0xff00_0000`.

The x86-64 bootstrap requires one RAM region beginning at GPA 0 with at least 2 MiB. `LongModeBootLayout::new` rejects:

- a non-zero RAM base;
- RAM smaller than `0x20_0000`;
- an entry at or beyond the 2 MiB identity-map extent;
- an entry inside the reserved page-table pages `0x1000..0x4000`;
- a zero stack pointer;
- a stack pointer above the mapped 2 MiB extent;
- a stack top that would overlap the reserved page-table area under the fixed bootstrap contract.

The flat deterministic long-mode proof uses entry `0x10000`; the ELF64 proof supplies validated ELF entry `0x10100`; both use stack pointer `0x1ff000`, well outside the page-table pages.

Page-table construction performs no raw pointer arithmetic. Three full 4 KiB pages at GPA `0x1000`, `0x2000`, and `0x3000` are zeroed through `GuestMemory::write`; only the first PML4, PDPT, and PD entries are then written. Their exact little-endian values are `0x2003`, `0x3003`, and `0x83`. This creates exactly one present/writable 2 MiB identity mapping for VA/GPA `0..0x20_0000`. No guest-controlled index, count, address, or page permission is used to size or select a host-memory slice.

This fixed mapping is not a general guest virtual-address translation facility. No arbitrary VA→GPA mapping API, dynamic page-table allocation, MMIO mapping, guest-supplied page-table parser, or page-fault recovery policy exists in this milestone.

## ELF64 loader safety

`Elf64GuestImage::parse` accepts only ELF64 little-endian x86-64 `ET_EXEC`. It validates the fixed header size and program-header entry size, converts and bounds the complete program-header table before traversing it, and never trusts a file offset or count as a Rust slice boundary without checked conversion and arithmetic.

For each `PT_LOAD`, validation requires non-zero memory size, `p_filesz <= p_memsz`, a file-backed range entirely inside the supplied bytes, and a guest range whose end cannot overflow. Segment alignment must be 0, 1, or a power of two; aligned segments must satisfy ELF offset/virtual-address congruence. The current fixed-map contract additionally requires `p_vaddr == p_paddr`, the entire segment inside `0..0x20_0000`, no overlap with bootstrap page tables `0x1000..0x4000`, and no overlap with another load segment.

An ELF entry is accepted only inside the file-backed portion of an executable `PT_LOAD`; an entry that exists only in a zero-filled BSS tail is rejected. Materialization occurs only after the whole image has been validated. File-backed bytes are copied through checked `GuestMemory::write`, and each BSS tail is explicitly zeroed before KVM memory registration. The deterministic fixture intentionally contains a non-empty BSS tail so this behavior is regression-tested rather than merely documented.

This loader does not perform relocation, load-bias selection, `ET_DYN`/PIE loading, dynamic linking/interpreter handoff, symbol resolution, section-driven loading, or arbitrary virtual-address mapping. Absence of those features is part of the safety boundary, not an implicit best-effort behavior.

## Long-mode vCPU state safety

`Vcpu::initialize_long_mode` first reads the vCPU's current KVM special-register state and then mutates only the fields required by the fixed bootstrap contract. It ORs the required `CR0.PE|CR0.PG`, `CR4.PAE`, and `EFER.LME|EFER.LMA` bits so unrelated inherited bits are not silently cleared, and sets `CR3` exactly to the validated PML4 GPA `0x1000`.

The segment state is not supplied by guest bytes. CS is a fixed present ring-0 64-bit code segment with selector `0x8`, base 0, limit `0xffff_ffff`, `L=1`, and `DB=0`. DS/ES/FS/GS/SS use the fixed present ring-0 data-segment contract with selector `0x10`, base 0, and the same limit. RIP and RSP come only from the validated `LongModeBootLayout`; RFLAGS is initialized with architectural bit 1 set and all remaining general-register fields begin from zero.

Failure of `KVM_GET_SREGS`, `KVM_SET_SREGS`, or `KVM_SET_REGS` is a named vCPU-operation error. The implementation does not retry a partially applied state sequence or claim transactional rollback. In deterministic proof paths, failure prevents a successful proof result.

## Deterministic executable proofs

The reviewed 36-byte flat x86-64 fixture is loaded at GPA/VA `0x10000`. It uses 64-bit-width instruction encodings, emits bytes `L`, `M`, `6`, and `4` through four byte-wide single-count OUT operations on the existing debug port `0xe9`, then executes `HLT`.

The ELF64 proof wraps the same architectural proof code inside the production `ET_EXEC` loader path. Its validated executable segment begins at GPA/VA `0x10000`, entry is `0x10100`, and the segment has a larger memory size than file size so the loader must zero BSS before execution.

Each proof uses an execution budget of exactly five completed exits. Success requires four serviced I/O exits in order followed by the typed terminal HLT report. The host-owned proof buffer must equal `LM64`; terminal RIP is `0x10024` for the flat fixture and `0x10124` for the ELF64 fixture. Budget exhaustion, another exit, malformed port-I/O metadata, an unsupported port operation, invalid executable metadata, or KVM entry/execution failure is not converted into milestone success.

KVM-aware Rust regressions follow the repository's general environment-sensitive convention. In addition, CI contains strict gates that directly run both `run-long-mode` and `run-elf64` with usable `/dev/kvm`, check the exact `LM64` output, check each exact terminal HLT RIP, and require architectural RFLAGS bit 1. Those gates fail if KVM is unavailable and provide evidence that the candidate actually executed the 64-bit guest paths rather than only validating pure construction.

## Port-I/O and execution-loop safety

For `KVM_EXIT_IO`, `data_offset` is an offset into the owned `kvm_run` mapping, never a trusted pointer. The vCPU layer checks integer conversion, checked `size * count`, checked range-end addition, and the final range against the mapping before any pointer arithmetic. OUT bytes are copied into owned Rust memory before device policy sees them.

For `KVM_EXIT_IO_IN`, device policy returns owned response bytes. The vCPU layer revalidates the current I/O metadata, requires IN direction, recomputes the checked range, and requires exact response length before copying bytes back into `kvm_run`.

KVM defines port-I/O completion as pending until userspace re-enters `KVM_RUN`. The execution loop therefore does not claim completed post-I/O state until a later completed exit. The explicit exit budget is checked before each run; only a successful completed KVM exit consumes one unit. Budget exhaustion remains a structured error, not a terminal guest report.

The long-mode and ELF64 milestones reuse this path unchanged; neither adds a new device model or raw I/O decoding path.

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

The long-mode bootstrap uses the same KVM register/special-register structures but does not turn those state snapshot APIs into a boot-state parser or migration format.

## VM and memory lifetime

Successful `KVM_CREATE_VM` and `KVM_CREATE_VCPU` results are immediately wrapped in `OwnedFd`. `Vm` owns registered guest RAM after successful slot registration. Before releasing RAM it attempts to unregister slot 0 with a zero-sized memory-region update. If unregister fails while an independent vCPU descriptor could keep the kernel VM alive, the userspace mapping is intentionally leaked rather than unmapped underneath a potentially live KVM slot.

## Not yet present

The repository now has one fixed 2 MiB x86-64 identity-mapped bootstrap and one bounded identity-mapped ELF64 `ET_EXEC` loader/execution path, but still has **no general virtual-memory subsystem** and no arbitrary guest address-space construction. It also has no ELF relocations, `ET_DYN`/PIE or dynamic-linker path, Linux boot protocol, MMIO model, APIC/interrupt controller, interrupt injection framework, virtio, SMP, dynamic device registration, disk backend, whole-VM/guest-memory/device snapshot orchestration, migration protocol, resumable execution, scheduler, exception recovery/injection policy, KVM-unknown recovery policy, fail-entry retry/placement policy, internal-error recovery policy, or system-event lifecycle policy.

Those responsibilities require separately selected milestones. The bounded ELF64 milestone does not authorize them implicitly.
