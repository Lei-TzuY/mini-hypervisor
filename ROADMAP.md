# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `70951389b42322a54ceeeebf7e88c6508f586255` through PR #112 (`Copy a ring3 byte through bounded fault fixups`). Exact merged-main verification is green across ordinary CI/MSRV and every permanent hosted-KVM workflow, including ring3/SYSCALL privilege execution, fault-safe copyin/copyout/usercopy, SMP/IPI/TLB-shootdown coverage, PCI/virtio-rng/virtio-blk execution and storage workflows.

The repository therefore integrates the Phase 73 foundation, x86-64/ELF64 execution, MMIO and controller-backed interrupts, direct/irqfd/eventfd asynchronous delivery, PCI/virtio execution, bounded SMP/IPI/timer/TLB-shootdown behavior, guest-owned ring3/TSS transitions, a bounded SYSCALL/SYSRET ABI, one-byte fault-safe copyin and copyout, and one combined one-byte `copy_byte(src, dst)` service backed by a guest-resident two-entry exception/fixup table.

The fixed one-byte usercopy/fixup phase is sealed. The integrated service proves a real CPL0 load followed by a real CPL0 store, exact recovery of both canonical-unmapped read and write faults through distinct table entries, exact `-EFAULT` returns, mapped destination readback, supervisor-only fault metadata and fail-closed handling for unlisted fault sites. Do not farm additional fixed pointers, byte values or static fixup entries merely to extend that phase.

## Selected milestone — bounded syscall-number dispatcher

The next boundary is a small executable syscall control plane rather than another copy primitive. RAX becomes an explicit syscall number while RDI/RSI remain arguments. One ring3 program must exercise two materially different CPL0 services and an unknown-number path through the existing SYSCALL/SYSRET boundary.

Acceptance contract:

- preserve exact merged-green base `70951389b42322a54ceeeebf7e88c6508f586255`, Rust 1.74 shipped-target MSRV, ordinary CI and every permanent hosted-KVM workflow already green there;
- `nr=0` executes fault-safe one-byte `copy_byte(src, dst)` with a real CPL0 load/store and its own two exact #PF/fixup sites;
- `nr=1` is a bounded privileged debug-putc service: ring3 supplies one byte in RDI, only CPL0 executes `OUT 0xe9`, and the ring3 guest contains no direct debug-port output;
- unknown syscall `0xff` emits `U` in CPL0 and returns exact `-ENOSYS` rather than aliasing either valid service;
- dispatcher selection is driven by RAX, preserves the user stack in R10, switches to the existing kernel stack and returns through SYSRETQ;
- deterministic ring3 sequence is good copy, kernel putc of `P`, bad-source copy, bad-destination copy, unknown syscall, then the existing DPL3 terminal gate;
- exact debug proof is `CPRWUD`: `C` follows the successful copy, `P` is produced by the privileged putc path, `R` and `W` follow the distinct read/write fault fixups, `U` proves unknown-number dispatch and `D` proves recovered return to ring3 before the terminal transition;
- mapped source `0xa100` starts at `0x6b`, destination `0xa101` starts at zero and both ring3/host readback must observe destination `0x6b` after the good copy;
- canonical-unmapped `0x400000` remains absent; bad-source records read #PF error `0x0` at RIP `0x12026` and fixup `0x12033`; bad-destination records write #PF error `0x2` at RIP `0x12029` and fixup `0x12040`;
- a supervisor-only two-entry table maps those fault sites to observations `0xb000` and `0xb040`; unlisted page-fault RIPs remain fail-closed rather than receiving a generic recovery;
- exact syscall returns are copy `0`, putc `0`, both bad copies `-EFAULT` and unknown `-ENOSYS`;
- the terminal ring3 frame remains RIP `0x1108a`, CS `0x23`, RFLAGS `0x202`, RSP `0x1fd000`, SS `0x1b`; final ring0 terminal state remains on the bounded kernel stack with IF clear and HLT RIP `0x13005`;
- user data remains U/S+writable while dispatcher code, page-fault handler and fault metadata remain supervisor-only; the bad-pointer PD entry remains non-present;
- KVM-aware integration must validate service selection, all six exact I/O exits, return values, mapped byte state, both #PF observations/fixups, the two-entry table, MSRs/PTEs and terminal frame/state;
- the permanent `syscall-dispatcher` hosted-KVM workflow must execute the standalone binary with a bounded timeout and hard-check `CPRWUD`, all five syscall returns, both distinct #PF error codes/fixups/table entries, ABI MSRs, mapped/unmapped permissions and terminal state; `/dev/kvm` is mandatory and there is no skip path;
- formatter, Clippy, MSRV, dispatcher branch offsets, machine-code service selection, #PF/fixup, SYSRET, proof or hosted-KVM failures remain hard failures and must not be hidden by changed expectations.

Implementation is in progress on `milestone/syscall-dispatcher` / PR #113. The production dispatcher, standalone executable and KVM-aware integration are implemented. Exact head `f6cb143ee4a7ef02151eaa7cee998d6765413783` passed ordinary CI #718, Rust 1.74 MSRV and all pre-existing permanent hosted-KVM workflows; its hosted-KVM test `syscall_number_dispatches_copy_putc_faults_and_unknown_service` executed successfully rather than taking an environment skip. The candidate now also includes this synchronized roadmap and a dedicated permanent `syscall-dispatcher` hosted-KVM workflow, so the new exact head must pass all gates before integration.

## Scope boundary

This milestone deliberately does **not** add:

- a general or dynamically registered syscall table, syscall ABI versioning, arbitrary syscall numbers or a userspace I/O privilege mechanism;
- multibyte/cross-page copies, partial-progress semantics, vectored I/O, mmap, demand paging, COW, allocation or signals;
- process/task objects, scheduling, multiple user address spaces, per-process CR3 ownership or multi-vCPU user execution;
- filesystem/VFS abstractions, file descriptors, sockets or a new storage/device transport;
- SMEP/SMAP, PKU, kernel preemption or a general exception policy;
- new PCI/MMIO/virtio/SMP capability, performance or latency claims.

## Promotion rule

After the bounded dispatcher is integrated and exact merged-`main` ordinary CI plus every permanent hosted-KVM workflow are green, seal the fixed `{copy_byte, putc, unknown}` dispatch proof rather than adding more hard-coded syscall numbers.

The next architecture audit must choose a materially different user/kernel boundary. Strong candidates are a bounded multi-byte/cross-page user-copy contract only if partial-progress and fault semantics are specified and executable, a minimal task/process/address-space ownership model, or a syscall-to-an-already-integrated device/storage capability that proves a genuine cross-layer service boundary. Do not promote merely by extending the dispatcher switch with more fixed cases or enlarging the existing static fixup table.
