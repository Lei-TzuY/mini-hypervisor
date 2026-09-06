# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `198adb9eee082fb2370a0b739ea000714240f53c` through PR #108 (`Enter ring3 through a guest-loaded TSS privilege boundary`). The repository integrates the Phase 73 foundation, x86-64/ELF64 execution, userspace and virtual MMIO, controller-backed interrupts, direct and irqfd/eventfd asynchronous delivery, PCI/virtio-rng/virtio-blk paths, a bounded two-vCPU SMP control/data plane, guest-originated xAPIC IPIs, targeted MSI, an AP-owned local LAPIC timer, one fixed cross-vCPU TLB shootdown, and one guest-owned ring3/TSS privilege boundary.

The ring3/TSS phase proves a real x86-64 CPL3→CPL0 boundary with guest-backed GDT/IDT/TSS state, guest `ltr`, user/supervisor page ownership, TSS RSP0 stack switching, DPL3 interrupt gates, `iretq` return to ring3, and a second terminal privilege transition. Its permanent hosted-KVM workflow and exact merged-main ordinary CI are green.

That fixed one-user-context ring3/TSS phase is sealed. Do not farm extra DPL3 vectors, selector variants, or duplicate user-context fixtures merely to extend the phase number.

## Selected milestone — bounded x86-64 SYSCALL/SYSRET ring3 ABI

The next architecture boundary is a real fast system-call privilege path. This milestone reuses the integrated guest-owned ring3 address space and descriptor state, but exercises the architecturally different `SYSCALL`/`SYSRETQ` mechanism: EFER.SCE, STAR/LSTAR/SFMASK, RCX/R11 transfer, an explicit software kernel-stack switch because `SYSCALL` does not consult TSS RSP0, and return to the existing CPL3 context.

This remains deliberately one vCPU and one user context. It proves the privilege-entry ABI and return contract without claiming a process model, syscall-number dispatcher, scheduler, per-process CR3, signal delivery, or general user-pointer recovery.

Acceptance contract:

- preserve ordinary CI, Rust 1.74 shipped-target MSRV, and every permanent hosted-KVM workflow already green on exact merged `main`;
- preserve the integrated EFER value and set only SCE, then program exact STAR/LSTAR/SFMASK through the existing policy-bound bounded `KVM_SET_MSRS` path and require exact readback;
- fixed STAR must select kernel CS/SS `0x08/0x10` and SYSRET user CS/SS `0x23/0x1b`; fixed LSTAR is `0x12000`; fixed SFMASK clears IF only;
- enter the existing CPL3 context with user RIP `0x11000`, RSP `0x1fd000`, CS `0x23`, SS `0x1b`, and RFLAGS `0x202`;
- user `SYSCALL` must place return RIP `0x11017` in RCX and user RFLAGS `0x202` in R11 while leaving the user RSP unchanged;
- the LSTAR handler must preserve the user RSP, switch explicitly to supervisor-only kernel stack `0x1fe000`, observe CS/SS `0x08/0x10` and RFLAGS `0x2`, emit `S`, restore the user RSP, and execute `SYSRETQ`;
- after SYSRET the user must again observe CS/SS `0x23/0x1b`, then enter the existing DPL3 terminal gate;
- the terminal five-word hardware frame must independently prove user RIP `0x1102f`, CS `0x23`, RFLAGS `0x202`, RSP `0x1fd000`, and SS `0x1b`;
- the terminal kernel handler emits `D` and halts; exact debug-port proof is `SD` across two byte-wide OUT exits, ending at HLT RIP `0x13005`;
- user code and user stack PTEs remain U/S, while the LSTAR handler and syscall observation page remain supervisor-only;
- KVM-aware integration must independently validate MSR state, pre/post-SYSRET selectors, RCX/R11/RSP observation, kernel stack state, terminal user frame, page permissions, terminal ring0 state, and exact proof;
- permanent workflow `Strict KVM SYSCALL SYSRET privilege ABI` must execute the standalone proof on hosted KVM with a bounded timeout and may not skip `/dev/kvm`, weaken selectors/MSRs/page-permission checks, or retry a failed privilege transition into success;
- formatter, Clippy, MSRV, MSR policy/readback, selector, stack, page-permission, execution-order, proof, and hosted-KVM failures remain hard failures.

Implementation is in progress on `milestone/syscall-sysret-abi` / PR #109. The pre-governance candidate `1c152e2120e4b2b68c60141855389827962066ac` passed ordinary CI #682 and all currently applicable permanent hosted-KVM workflows after formatter and Clippy construction issues were fixed without changing runtime semantics. The dedicated permanent SYSCALL/SYSRET workflow and this roadmap synchronization are part of the same coherent milestone; no capability is considered integrated until the final exact candidate passes every applicable workflow, remains current with `main`, completes the normal review/merge audit, and the merged-main permanent SYSCALL/SYSRET workflow succeeds.

## Scope boundary

This milestone deliberately does **not** add:

- a syscall-number dispatch table, kernel service layer, process/task model, scheduler, multiple user contexts, or per-process CR3/address spaces;
- SYSENTER/SYSEXIT, arbitrary STAR layouts, per-vCPU TSS refactoring, SMP user tasks, or multi-vCPU syscall delivery;
- SMEP/SMAP, TSS I/O-bitmap policy, user-mode port-I/O, signal delivery, #PF/#GP recovery, copyin/copyout, demand paging, or fault-safe user pointers;
- new MMIO, timer, PCI/virtio, storage, DMA/IOMMU, migration, performance, latency, persistence, or durability claims.

## Promotion rule

After the bounded SYSCALL/SYSRET path is integrated and exact merged-`main` ordinary CI plus every permanent workflow are green, seal this fixed one-syscall entry/return proof rather than adding syscall-number or STAR-selector variants.

The next architecture audit should select a genuinely higher-order boundary. Strong candidates are a bounded fault-safe user-memory/copyin boundary that can prove recovery from a bad ring3 pointer without corrupting kernel execution, or a reusable per-vCPU privilege/TSS model only if it is exercised by real multi-vCPU ring3 execution. A syscall dispatch table is worthwhile only when it introduces a second executable service with a materially different data/validation path rather than naming inflation.
