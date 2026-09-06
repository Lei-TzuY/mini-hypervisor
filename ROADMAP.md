# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `560664bb7570549cf453765b0e4c65d13909094c` through PR #111 (`Recover bad ring3 destinations through fault-safe copyout`). Exact merged-main verification is green across ordinary CI/MSRV and every permanent hosted-KVM workflow, including the integrated copyin and copyout proofs, ring3/SYSCALL privilege paths, SMP/IPI/TLB-shootdown coverage, PCI/virtio-rng/virtio-blk execution and storage workflows.

The repository therefore integrates the Phase 73 foundation, x86-64/ELF64 execution, MMIO and controller-backed interrupts, direct/irqfd/eventfd asynchronous delivery, PCI/virtio execution, bounded SMP/IPI/timer/TLB-shootdown behavior, guest-owned ring3/TSS transitions, a bounded SYSCALL/SYSRET ABI, and both one-byte fault-safe user-memory directions.

The fixed one-byte copyin/copyout pair is sealed. Copyin proves a real CPL0 load from canonical-unmapped `0x400000` generates guest #PF with read error `0x0`, records supervisor-only fault metadata, rewrites the unique saved RIP to its fixup and returns exact `-EFAULT`. Copyout proves the corresponding real CPL0 store generates write error `0x2`, recovers through its own fixup and preserves ring3/host readback on the mapped path. Do not farm additional fixed bad pointers, byte values or one-site read/write variants merely to extend those phases.

## Selected milestone — bounded fault-safe `copy_byte(src, dst)` service

The next boundary is a real data service that consumes both integrated directions in one SYSCALL and therefore requires more than two unrelated hard-coded handlers. One bounded `copy_byte(src, dst)` performs a CPL0 user-memory load followed by a CPL0 user-memory store and recovers either fault through a guest-resident two-entry exception/fixup table selected by the page-fault handler itself.

Acceptance contract:

- preserve exact merged-green `main` `560664bb7570549cf453765b0e4c65d13909094c`, Rust 1.74 shipped-target MSRV, ordinary CI and every permanent hosted-KVM workflow already green there;
- retain the integrated SYSCALL/SYSRET ABI and existing copyin/copyout permanent proofs unchanged;
- mapped source `0xa100` starts at `0x6b`, mapped destination `0xa101` starts at zero, and the CPL0 service must execute a real `movzx eax,[rdi]` followed by a real `mov [rsi],al` so ring3 and host inspection both observe destination `0x6b` with return zero;
- canonical-unmapped `0x400000` remains absent from the page tables for both negative calls;
- exact read and write fault RIPs are `0x1200d` and `0x12010`; exact recovery RIPs are `0x1201b` and `0x12021`;
- a supervisor-only table at `0xb100` contains exactly two 24-byte entries `(fault RIP, fixup RIP, observation address)` mapping read→`0xb000` and write→`0xb040` observations;
- vector-14 handling must select the table entry from the architectural saved RIP. An unlisted fault RIP emits `X` and HLTs fail-closed rather than applying a generic fixup;
- the selected observation records CR2, error code, saved RIP, CS, RFLAGS and resolved fixup; only the saved RIP is rewritten, the architectural #PF error-code word is discarded and IRETQ returns to the selected service fixup;
- bad-source must record CR2 `0x400000`, error `0x0`, RIP `0x1200d`, CS `0x8`, RFLAGS `0x10002`, fixup `0x1201b` and return exact `-EFAULT`;
- bad-destination must record CR2 `0x400000`, error `0x2`, RIP `0x12010`, CS `0x8`, RFLAGS `0x10002`, fixup `0x12021` and return exact `-EFAULT`;
- exact debug proof is `CRWD`: `C` only after the good read+write pair, `R` only after the read-fault fixup, `W` only after the write-fault fixup and `D` only after the recovered third SYSRET returns to ring3 and enters the existing DPL3 terminal gate;
- terminal user frame remains RIP `0x11060`, CS `0x23`, RFLAGS `0x202`, RSP `0x1fd000`, SS `0x1b`; ring0 terminal state remains IF-clear with HLT RIP `0x13005`;
- the source/destination/result page remains U/S+writable, page-fault handler/table/observations remain supervisor-only, and the bad-pointer PD entry remains non-present;
- KVM-aware integration independently validates all four OUT exits, three returns, ring3/host byte readback, both fault observations, both table entries, MSRs, page permissions, terminal frame/state and HLT report;
- the permanent `fault-safe-usercopy` hosted-KVM workflow must execute the standalone binary with a bounded timeout and hard-check `CRWD`, both distinct #PF error codes/fixups, both table entries, mapped/unmapped permissions, ABI MSRs and terminal state; `/dev/kvm` is mandatory and no skip path is allowed;
- formatter, Clippy, MSRV, machine-code offsets, table selection, #PF frame/fixup, SYSRET, proof and hosted-KVM failures remain hard failures and must not be hidden by changed expectations.

Implementation is in progress on `milestone/fault-safe-usercopy-service` / PR #112. The production service, standalone executable and KVM-aware integration are implemented. Exact head `3cf77fdff7d8bf17cf4123d5ddd1ce5ca2592030` already passed ordinary CI/MSRV and every pre-existing permanent hosted-KVM workflow. The candidate now also includes this synchronized roadmap and a dedicated permanent `fault-safe-usercopy` hosted-KVM workflow; the new exact head must pass them before integration.

## Scope boundary

This milestone deliberately does **not** add:

- general-length copies, loops, cross-page partial-copy semantics, vectored copies, pinning, demand paging, mmap, COW, page allocation, signals or page-fault retry;
- dynamic exception-table registration or an unbounded generic kernel exception framework; the table is bounded to the two proven service sites;
- syscall-number dispatch, process/task objects, scheduling, multiple user contexts, per-process CR3/address spaces or multi-vCPU user execution;
- SMEP/SMAP, PKU, arbitrary exception policy, user-mode port I/O or kernel preemption;
- new MMIO/device/storage/SMP capability, performance or latency claims.

## Promotion rule

After the two-entry usercopy service is integrated and exact merged-`main` ordinary CI plus every permanent workflow are green, seal the fixed one-byte usercopy/fixup phase rather than adding more static table entries, pointers or values.

The next architecture audit should promote to a materially different user/kernel boundary. Strong candidates are a minimal syscall-number dispatcher with two genuinely distinct executable services, a bounded multi-byte/cross-page copy model only if partial-progress and fault semantics are specified and proven, or a higher-order process/address-space boundary. Do not promote merely by making the fixup table dynamically sized or by duplicating the same one-byte service.
