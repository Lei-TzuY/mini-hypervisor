# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `fdeb42d0576fe048f234fb09675d5adb70a24e8f` through PR #113 (`Dispatch bounded ring3 syscalls by number`). The merged mainline preserves Rust 1.74 shipped-target MSRV, ordinary CI and the repository's permanent hosted-KVM proof suite across the previously integrated x86-64/ELF64, MMIO/interrupt, PCI/virtio, SMP/TLB, ring3/SYSCALL and fault-safe usercopy surfaces.

The repository therefore integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI; one-byte fault-safe copyin, copyout and combined usercopy; and a bounded RAX-selected syscall dispatcher.

PR #113 seals the fixed dispatcher proof. RAX selects exactly `{copy_byte, debug_putc}` or the unknown-number path, the two valid services execute in CPL0, unknown `0xff` returns exact `-ENOSYS`, and dispatcher-local copy faults use their own exact two-entry #PF/fixup table. Do not farm additional fixed syscall numbers merely to grow the switch.

## Selected milestone — bounded cross-page usercopy with partial progress

The next user/kernel boundary is a real multi-byte copy contract rather than another dispatcher case. A four-byte CPL0 copy loop crosses 4 KiB user-page boundaries, exposes a single reusable load site and store site, and reports the exact number of bytes committed before a source-read or destination-write page fault.

Acceptance contract:

- preserve exact merged-green base `fdeb42d0576fe048f234fb09675d5adb70a24e8f`, Rust 1.74 shipped-target MSRV, ordinary CI and every permanent hosted-KVM workflow already green there;
- copy length is exactly four bytes; R8 is the architectural progress count and increments only after one byte load and one byte store both complete;
- retain exactly one loop load site at `LSTAR+19`, one loop store site at `LSTAR+24` and two guest-resident fixup-table entries rather than unrolling four copies or farming static fault sites;
- both exact fault sites resolve to the common return fixup `LSTAR+37`; page-fault recovery preserves R8 and returns the completed-byte count through RAX/SYSRETQ;
- the successful case copies source `0x20ffe` to destination `0x22ffe`, crosses present user pages, returns `4`, and leaves exact destination `[0x11,0x22,0x33,0x44]`;
- the source-fault case starts at source `0x24ffe`, faults on byte three at non-present page `0x25000`, returns `2`, and leaves destination `[0x55,0x66,0,0]`;
- the destination-fault case starts at source `0x28ffe` and destination `0x2affe`, faults on byte three at non-present page `0x2b000`, returns `2`, and leaves destination `[0x99,0xaa,0,0]`;
- the exact read fault is CR2 `0x25000`, error `0x0`, RIP `0x12013`; the exact write fault is CR2 `0x2b000`, error `0x2`, RIP `0x12018`; both have kernel CS `0x8`, saved RFLAGS `0x10046` and fixup RIP `0x12025`;
- `cmp r8,r8` immediately precedes the unique loop load/store sequence so the fault frame deterministically contains architectural bit 1 plus PF, ZF and RF while IF remains masked by the SYSCALL contract;
- all present data pages remain user+writable; exactly `0x25000` and `0x2b000` are user+writable but non-present; the syscall service, #PF handler and fault metadata remain supervisor-only;
- unmatched page-fault RIPs remain fail-closed and must never receive generic recovery;
- one ring3 program executes good copy, source-fault copy and destination-fault copy, returns to ring3 after each through the existing DPL3 return gate, then terminates through the existing terminal gate; exact debug proof is `KKKD`;
- KVM-aware integration must validate return counts, source/destination backing state, both exact #PF observations, both fixup entries, user/supervisor PTEs, ABI MSRs and the terminal privilege frame/state;
- a permanent `cross-page-usercopy` hosted-KVM workflow must execute the standalone binary with a bounded timeout, require `/dev/kvm` with no skip path, and hard-check proof, partial progress, fault metadata/fixups, mapping permissions, terminal state and ABI MSRs;
- formatter, Clippy, MSRV, loop/fixup machine code, #PF recovery, partial-progress accounting, SYSRET, proof or hosted-KVM failures remain hard failures and must not be hidden by changing expectations.

Implementation is in progress on `milestone/cross-page-usercopy` / PR #114. The production service, standalone executable and KVM-aware integration were implemented first. Exact production head `31468460ce16d76421d31968f25248f5063f2185` passed ordinary CI #729 after formatter-only corrections; that run executed `cross_page_copy_reports_exact_partial_progress_after_read_and_write_faults ... ok` on the hosted KVM runner rather than taking its environment skip path. The dedicated permanent workflow and this roadmap synchronization are subsequent governance/evidence changes, so that pre-governance green run is not the final merge candidate. The new exact head must re-pass ordinary CI, the permanent cross-page proof and all other triggered workflows before integration.

## Scope boundary

This milestone deliberately does **not** add:

- arbitrary copy lengths, large-buffer throughput claims, vectored I/O or an optimized generic `copy_from_user`/`copy_to_user` subsystem;
- mmap, demand paging, copy-on-write, allocation, swapping, signals or a general page-fault policy;
- process/task objects, scheduling, multiple user address spaces, per-process CR3 ownership or multi-vCPU user execution;
- filesystem/VFS abstractions, file descriptors, sockets, storage syscalls or a new device transport;
- SMEP/SMAP, PKU, kernel preemption or security claims beyond the explicitly executed U/S and fault-fixup invariants;
- new PCI/MMIO/virtio/SMP capability, performance or latency claims.

## Promotion rule

After this four-byte cross-page contract is integrated and exact merged-`main` ordinary CI plus every triggered permanent hosted-KVM workflow are green, seal the fixed-length partial-progress proof rather than adding more fixed lengths, pointer placements or static fault cases.

The next architecture audit must choose a materially different boundary. Prefer composing the proven partial-progress copy semantics into the bounded syscall control plane only if that creates a real caller-visible multi-byte service with explicit ABI/error behavior, or promote to task/address-space ownership when a bounded executable per-process CR3/ownership slice is ready. A syscall-to-an-already-integrated storage/device capability is also valid only when it proves a genuine cross-layer service path. Do not promote by merely increasing the copy length, adding more hard-coded dispatcher cases or enlarging the existing static fixup table.
