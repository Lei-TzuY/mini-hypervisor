# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `c06ddd616ffc06b572635f380113a1dbde374d3f` through PR #109 (`Execute a bounded SYSCALL/SYSRET ring3 ABI`). All ordinary and permanent hosted-KVM workflows for that exact merged commit are green. The repository integrates the Phase 73 foundation, x86-64/ELF64 execution, userspace and virtual MMIO, controller-backed interrupts, direct/irqfd/eventfd asynchronous delivery, PCI/virtio-rng/virtio-blk paths, a bounded two-vCPU SMP control/data plane, guest-originated xAPIC IPIs, targeted MSI, AP-owned LAPIC timer execution, cross-vCPU TLB shootdown, guest-owned ring3/TSS privilege transitions, and a bounded one-context SYSCALL/SYSRET ABI.

The SYSCALL/SYSRET phase proves EFER.SCE plus exact STAR/LSTAR/SFMASK state, RCX/R11 transfer, an explicit supervisor kernel-stack switch, SYSRETQ back to CPL3, preserved user/supervisor page ownership, and a second DPL3 terminal transition. That fixed entry/return ABI is sealed. Do not farm syscall-number placeholders, STAR selector variants, or duplicate one-service fixtures merely to extend the phase number.

## Selected milestone — fault-safe one-byte user copyin recovery

The next architecture boundary is safe recovery from a bad ring3-provided pointer while already executing in the integrated CPL0 SYSCALL handler. This milestone does not merely range-check the pointer. A fixed canonical but unmapped pointer must make the actual kernel load instruction generate a guest page fault, and the guest kernel must recover through one bounded DPL0 #PF gate and one fixed copyin fixup before returning `-EFAULT` to the same ring3 context.

Acceptance contract:

- preserve exact merged `main` `c06ddd616ffc06b572635f380113a1dbde374d3f`, Rust 1.74 shipped-target MSRV, ordinary CI, and every permanent hosted-KVM workflow already green there;
- reuse the integrated `LongModePrivilegeLayout`, TSS RSP0 boundary, user/supervisor page ownership and exact STAR/LSTAR/SFMASK ABI; do not add a syscall-number dispatcher, process model, scheduler or per-process CR3;
- fixed good user pointer `0xa100` must reside on an existing U/S page, contain byte `0x5a`, and return zero-extended `0x5a` through SYSRET;
- fixed bad user pointer `0x400000` must be canonical and remain unmapped; the PD entry covering it must have Present clear both before and after execution;
- the CPL0 copy instruction is uniquely fixed at RIP `0x1200d`; the only recovery target is fixup RIP `0x1201a`;
- install one DPL0 64-bit interrupt gate for vector 14 targeting supervisor-only handler `0x14000`; existing DPL3 user and terminal gates remain unchanged;
- the page-fault handler must record CR2, error code, saved RIP/CS/RFLAGS to supervisor-only page `0xb000`, rewrite only saved RIP to `0x1201a`, discard the architectural #PF error-code word, set RAX to exact `-EFAULT` (`0xfffffffffffffff2`), then IRETQ to the fixed copyin continuation;
- the required fault observation is CR2 `0x400000`, error code `0` (non-present supervisor read), fault RIP `0x1200d`, CS `0x8`, and saved RFLAGS `0x10002`; bit 16 is required because x86 fault-class exceptions set RF in the pushed RFLAGS image;
- ring3 itself must store both syscall returns on its U/S page: first result `0x5a`, second result `0xfffffffffffffff2`;
- after the recovered bad copy, the same ring3 context must still enter the existing DPL3 terminal gate; the terminal frame must prove RIP `0x1102b`, CS `0x23`, RFLAGS `0x202`, RSP `0x1fd000`, SS `0x1b`;
- terminal ring0 state remains RSP `0x1fdfd8`, CS `0x8`, IF clear, and final HLT RIP `0x13005`; final architectural CR2 remains `0x400000`;
- exact debug-port proof is `GFD`: `G` only after the good copy completes, `F` only from the page-fault recovery handler, and `D` only after the recovered SYSRET returns to user and the user enters the terminal gate;
- the good/result page remains U/S while the #PF handler and #PF observation pages remain supervisor-only;
- KVM-aware integration must independently validate all three byte-wide OUT exits, good/bad return values, full #PF observation, terminal frame, MSRs, page permissions, absent bad-pointer PD entry, final CR2 and terminal report;
- a dedicated permanent hosted-KVM workflow must execute the standalone proof with a bounded timeout and may not skip `/dev/kvm`, map the bad pointer, weaken #PF/error/RF checks, or accept a host-side `KVM_EXIT_EXCEPTION` in place of guest IDT recovery;
- formatter, Clippy, MSRV, machine-code offset, MSR, page-table, #PF frame, fixup, SYSRET, terminal-frame, proof and hosted-KVM failures remain hard failures.

Implementation is in progress on `milestone/fault-safe-copyin` / PR #110. The first construction candidate established the production module, executable, crate wiring and KVM-aware integration. Rust 1.74 MSRV passed immediately; the first stable job stopped only at rustfmt. Those formatter differences plus deterministic unused-import and page-fault-handler test-offset issues were corrected without changing the fault site, fixup, pointer mapping or recovery semantics. No capability is integrated until the final exact candidate passes every applicable workflow, remains current with `main`, completes review/merge audit, and the merged-main permanent copyin workflow succeeds.

## Scope boundary

This milestone deliberately does **not** add:

- general-length copyin loops, cross-page partial-copy semantics, copyout, demand paging, mmap, COW, page allocation, user-pointer pinning, exception-table registries, #GP recovery or signal delivery;
- syscall-number dispatch, multiple services, process/task objects, scheduler, multiple user contexts, per-process address spaces or multi-vCPU user execution;
- SMEP/SMAP, PKU, TSS I/O bitmap policy, arbitrary exception gates, kernel preemption, user-mode port I/O or general fault policy;
- new MMIO, timer, PCI/virtio, storage, DMA/IOMMU, migration, persistence, performance or latency claims.

## Promotion rule

After fault-safe copyin is integrated and exact merged-`main` ordinary CI plus every permanent workflow are green, seal this one-byte one-fault-site proof rather than farming more fixed bad addresses or copy lengths.

The next architecture audit should select a materially higher-order user/kernel boundary. Strong candidates are a fault-safe copyout path if it introduces a distinct write-fault/recovery contract, a reusable bounded exception-table/fixup abstraction only if exercised by more than one real kernel access site, or a second syscall service only when it consumes the copyin boundary through materially different validation/data flow. General process management, multiple address spaces and multi-vCPU user execution remain separate frontiers.
