# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `20ec83c11dad0029b27a3912a438da6e9c9e7142` through PR #114 (`Report partial progress across cross-page usercopy faults`). The exact merged mainline preserves Rust 1.74 shipped-target MSRV, ordinary CI and every triggered permanent hosted-KVM workflow across the integrated x86-64/ELF64, MMIO/interrupt, PCI/virtio, SMP/TLB, ring3/SYSCALL and fault-safe usercopy surfaces.

The repository therefore integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall-number dispatcher; one-byte fault-safe copyin/copyout/usercopy; and a bounded four-byte cross-page CPL0 usercopy loop with exact partial-progress reporting.

PR #114 seals the fixed four-byte cross-page service. It uses one reusable load site and one reusable store site, exactly two guest-resident fixup entries, returns progress only after a byte load and store both complete, reports two committed bytes before either the non-present source page `0x25000` or destination page `0x2b000`, and is permanently exercised by the `cross-page-usercopy` hosted-KVM workflow. Do not farm additional fixed lengths or pointer placements merely to extend that phase.

## Selected milestone — expose partial-progress usercopy through syscall dispatch

The next boundary is caller-visible syscall composition rather than another standalone usercopy fixture. Syscall nr2 accepts `RDI=source`, `RSI=destination`, `RDX=length`, reuses the proven partial-progress semantics for lengths `1..=4`, and coexists with the integrated nr0 copy-byte, nr1 debug-putc and unknown-number behavior in one ring3 execution.

Acceptance contract:

- preserve exact merged-green base `20ec83c11dad0029b27a3912a438da6e9c9e7142`, Rust 1.74 shipped-target MSRV, ordinary CI and every permanent hosted-KVM workflow green on PR #114 merged main;
- add syscall nr2 with ABI `RDI=source`, `RSI=destination`, `RDX=length`; valid length is exactly `1..=4`;
- successful nr2 returns the number of bytes committed; a recoverable source-read or destination-write page fault returns the exact number of bytes whose load and store both completed before the fault;
- length `0` and lengths above `4` return exact `-EINVAL` without touching user memory;
- progress lives in R8 and increments only after the current byte's load and store both complete;
- retain one reusable nr2 range-load site and one reusable range-store site with exactly two nr2 fixup entries; do not unroll fault sites by length or outcome;
- preserve the integrated nr0/nr1 fault-site offsets and common-return layout in the compatibility dispatcher image; nr2 is reached through a fixed-size trampoline and append-only service code;
- one ring3 program must prove: length4 success returns `4`, source fault returns `2`, destination fault returns `2`, length1 success returns `1`, length0 returns `-EINVAL`, length5 returns `-EINVAL`, legacy nr0 succeeds, legacy nr1 succeeds, and unknown number returns exact `-ENOSYS`;
- exact debug proof is `MFFMIICPUD`: `M` marks full/short range success, `F` partial range fault, `I` invalid length, `C` legacy copy-byte, `P` legacy debug-putc, `U` unknown-number handling, and `D` terminal privilege entry;
- the good four-byte destination must be `[0x11,0x22,0x33,0x44]`; source-fault destination `[0x55,0x66,0,0]`; destination-fault destination `[0x99,0xaa,0,0]`; one-byte short destination `[0xde,0,0,0]`; legacy nr0 destination byte `0x6b`;
- exact nr2 read fault is CR2 `0x25000`, error `0x0`, RIP `0x12078`; exact nr2 write fault is CR2 `0x2b000`, error `0x2`, RIP `0x1207d`; both use kernel CS `0x8`, saved RFLAGS `0x10046` and common fixup RIP `0x12098`;
- nr2 has exactly two fixup entries: read fault → `0x12098` with observation `0xb000`, write fault → `0x12098` with observation `0xb040`;
- all present user data pages remain present+user+writable; `0x25000` and `0x2b000` remain user+writable but non-present; the syscall service, #PF handler and fault metadata remain present supervisor-only;
- preserve the SYSCALL ABI MSRs: EFER.SCE set, STAR `0x0013000800000000`, LSTAR `0x12000`, SFMASK `0x200`;
- preserve the ring3 terminal frame selectors/state and the terminal CPL0 HLT path; unmatched page-fault RIPs remain fail-closed;
- KVM-aware integration must independently validate all return values, destinations, faults, fixups, proof bytes, user/supervisor PTE ownership, ABI MSRs and terminal privilege state;
- a permanent `syscall-partial-copy` hosted-KVM workflow must execute the standalone binary under a bounded timeout, require `/dev/kvm` with no skip path, and hard-check the same syscall/fault/mapping/MSR/terminal invariants;
- formatter, Clippy, MSRV, dispatcher compatibility, nr2 loop/fixup code, partial-progress accounting, invalid-length handling, legacy nr0/nr1 behavior, SYSRET, proof or hosted-KVM failures remain hard failures and must not be hidden by altered expectations.

Implementation is in progress on `milestone/syscall-partial-copy` / PR #115. The production nr2 service, standalone executable and KVM-aware integration passed the first compiler/MSRV/executable pass at exact head `6f19cadfdd087d7e41d1fc6a2dd65985ced18e13`; ordinary CI and all previously triggered permanent workflows were green, and the KVM-aware partial-copy integration executed successfully on hosted KVM. The standalone verifier has since been expanded to expose fixup, PTE, terminal and MSR observations, and a dedicated permanent `syscall-partial-copy` hosted-KVM workflow has been added. Those governance/evidence changes create a new exact head, so the earlier green head is not final merge evidence. The final candidate must re-pass ordinary CI, the new permanent proof and every other triggered workflow before integration.

## Scope boundary

This milestone deliberately does **not** add:

- arbitrary or unbounded copy lengths, vectored I/O, large-buffer throughput claims or an optimized generic usercopy subsystem;
- allocation, mmap, demand paging, copy-on-write, swapping, signals or a general page-fault policy;
- task/process objects, scheduling, multiple user address spaces, per-process CR3 ownership or multi-vCPU user execution;
- filesystem/VFS abstractions, file descriptors, sockets, storage/device syscalls or a new device transport;
- SMEP/SMAP, PKU, kernel preemption or security claims beyond the explicitly executed U/S, privilege and fault-fixup invariants;
- new PCI/MMIO/virtio/SMP capability, performance or latency claims.

## Promotion rule

After nr2 partial-progress copy is integrated and exact merged-`main` ordinary CI plus every triggered permanent hosted-KVM workflow are green, seal this bounded syscall composition rather than adding nr3/nr4 clones or more hard-coded copy lengths.

The next architecture audit must choose a materially different boundary. Prefer task/address-space ownership when a bounded executable per-process CR3/ownership slice is ready, or compose the syscall control plane with an already-integrated storage/device capability only when it produces a genuine caller-visible service path with explicit resource/error semantics. Do not promote by merely increasing copy length, adding more fixed syscall numbers, duplicating fault sites or enlarging the static fixup table.