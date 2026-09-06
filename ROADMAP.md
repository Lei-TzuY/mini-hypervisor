# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `51c6e89c6de7c3c9e3d0c9ab48e46982b4ec21da` through PR #110 (`Recover bad ring3 pointers through a fault-safe copyin boundary`). Exact merged-main verification is green: 19 check-runs completed successfully, including ordinary CI/MSRV, the permanent fault-safe-copyin hosted-KVM proof, the integrated ring3/SYSCALL privilege gates, SMP/IPI/TLB-shootdown coverage, PCI/virtio-rng/virtio-blk execution paths, and the existing storage workflows.

The repository therefore integrates the Phase 73 foundation, x86-64/ELF64 execution, userspace and virtual MMIO, controller-backed interrupts, direct/irqfd/eventfd asynchronous delivery, PCI/virtio-rng/virtio-blk paths, bounded SMP/IPI/timer/TLB-shootdown execution, guest-owned ring3/TSS privilege transitions, a bounded SYSCALL/SYSRET ABI, and one real fault-safe one-byte copyin boundary.

The copyin phase is sealed. Its executable contract proves that a real CPL0 load from canonical-unmapped user pointer `0x400000` generates guest #PF vector 14, records CR2/error/RIP/CS/RFLAGS in supervisor-only memory, rewrites only the unique copy site to a fixed fixup, returns exact `-EFAULT` through the same SYSRET path, and resumes the original ring3 context. Do not farm additional fixed bad pointers or copy lengths merely to extend that phase.

## Selected milestone — fault-safe one-byte user copyout recovery

The next architecture boundary is the write side of the same user/kernel data boundary. This milestone must not simulate success with host memory writes or prevalidate the bad pointer. The CPL0 SYSCALL handler itself must execute one real byte store through the ring3-provided destination, prove success by ring3 readback on the mapped path, and recover from the architecturally distinct write page fault on the canonical-unmapped path.

Acceptance contract:

- preserve exact merged `main` `51c6e89c6de7c3c9e3d0c9ab48e46982b4ec21da`, Rust 1.74 shipped-target MSRV, ordinary CI, and every permanent hosted-KVM workflow already green there;
- reuse the integrated `LongModePrivilegeLayout`, TSS RSP0 boundary, U/S page ownership and exact STAR/LSTAR/SFMASK ABI; do not add a syscall-number dispatcher, process model, scheduler or per-process CR3;
- fixed good user destination `0xa100` must reside on an existing U/S writable page, begin as zero, and be changed only by the guest CPL0 copyout store to byte `0xa5`;
- ring3 must independently load that byte back after SYSRET and record `0xa5`; host-side guest-memory inspection must agree, while the good syscall return is exact zero;
- fixed bad user destination `0x400000` must remain canonical and unmapped; the PD entry covering it must have Present clear before and after execution;
- the unique CPL0 store instruction is `mov byte ptr [rdi], 0xa5` at RIP `0x1200d`; the only recovery target is fixup RIP `0x1201a`;
- install one DPL0 64-bit interrupt gate for vector 14 targeting supervisor-only handler `0x14000`; existing DPL3 user and terminal gates remain unchanged;
- the required bad-store observation is CR2 `0x400000`, page-fault error code `0x2` (P=0, W/R=1, U/S=0), fault RIP `0x1200d`, CS `0x8`, and saved RFLAGS `0x10002`; the write bit is mandatory and distinguishes this phase from copyin;
- the #PF handler must record CR2/error/saved RIP/CS/RFLAGS to supervisor-only page `0xb000`, rewrite only saved RIP to `0x1201a`, set exact `-EFAULT` (`0xfffffffffffffff2`), discard the architectural #PF error-code word, and IRETQ to the existing SYSRETQ continuation;
- ring3 must store both syscall returns, preserve the good-byte readback, then enter the existing DPL3 terminal gate after the recovered bad copy;
- the terminal frame must prove RIP `0x11032`, CS `0x23`, RFLAGS `0x202`, RSP `0x1fd000`, SS `0x1b`; terminal ring0 state remains RSP `0x1fdfd8`, CS `0x8`, IF clear, final CR2 `0x400000`, and HLT RIP `0x13005`;
- exact debug-port proof is `WFD`: `W` only after the real mapped store, `F` only from the page-fault recovery handler, and `D` only after the recovered SYSRET returns to user and the user enters the terminal gate;
- the good destination PTE must have both U/S and Writable set; the #PF handler and observation pages remain supervisor-only; the bad pointer remains unmapped;
- KVM-aware integration must independently validate all three byte-wide OUT exits, good/bad returns, ring3 and host readbacks, full #PF observation, terminal frame, MSRs, page permissions, absent bad-pointer PD entry, final CR2 and terminal report;
- a dedicated permanent hosted-KVM workflow must execute the standalone proof with a bounded timeout and may not skip `/dev/kvm`, map the bad pointer, weaken the W/R bit/error/RF checks, or accept a host-side `KVM_EXIT_EXCEPTION` in place of guest IDT recovery;
- formatter, Clippy, MSRV, machine-code offset, MSR, page-table, #PF frame, fixup, SYSRET, terminal-frame, proof and hosted-KVM failures remain hard failures.

Implementation is in progress on `milestone/fault-safe-copyout` / PR #111. The production path, standalone executable, crate wiring and KVM-aware integration are implemented. Early candidates preserved Rust 1.74 and all pre-existing permanent hosted-KVM gates; ordinary CI initially stopped only on rustfmt, which was corrected without changing the store site, fault site, fixup, pointer mapping or architectural expectations. A dedicated permanent `fault-safe-copyout` hosted-KVM workflow is now part of the candidate and must pass on the final exact head before merge.

## Scope boundary

This milestone deliberately does **not** add:

- general-length copyout loops, cross-page partial-write semantics, copyin/copyout batching, demand paging, mmap, COW, page allocation, user-pointer pinning, signal delivery or page-fault retry;
- a generalized exception-table/fixup registry in this slice; abstraction is only justified after both real copyin and copyout sites are integrated and can exercise it without reducing coverage;
- syscall-number dispatch, multiple services, process/task objects, scheduler, multiple user contexts, per-process address spaces or multi-vCPU user execution;
- SMEP/SMAP, PKU, arbitrary exception gates, kernel preemption, user-mode port I/O or general fault policy;
- new MMIO, timer, PCI/virtio, storage, DMA/IOMMU, migration, persistence, performance or latency claims.

## Promotion rule

After fault-safe copyout is integrated and exact merged-`main` ordinary CI plus every permanent workflow are green, seal the one-byte read/write fault-recovery pair rather than farming more fixed pointers, values or lengths.

The next architecture audit should prefer a higher-order boundary. A strong candidate is a small shared bounded exception/fixup abstraction only if both integrated copyin and copyout paths are migrated to and executable through it with unchanged fault evidence. Otherwise select a second real syscall/data service or another user/kernel boundary that consumes these primitives through materially different control/data flow. General process management, multiple address spaces and multi-vCPU user execution remain separate frontiers.
