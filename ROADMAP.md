# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `d656ea0370828c6c47ea39c7a3b83d058b4b3da1` through PR #120 (`Wake a blocked guest task from an external timer`). The exact merged main is the green integration base for the next slice.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; two isolated ring3 address spaces; opt-in slot-0 dirty-page tracking; bounded guest-owned A→B→A task context switching; host-timer task preemption; and one bounded Runnable→Blocked→Runnable task lifecycle with external wakeup.

PR #118 seals the fixed cooperative two-task context switch. PR #119 seals host-timer preemption. PR #120 seals task lifecycle state: task A blocks through DPL3 vector `0x7e`, the existing scheduler dispatches B, B arms an external wake through vector `0x7f`, a host one-shot GSI0/vector `0x40` transition marks A Runnable, and the existing scheduler restores A. Exact hosted-KVM proof `KAPWBRD`, direct Blocked/Runnable state observations, A/B saved contexts, CR3/R12 ownership, physical stack markers and supervisor-only context PTEs are permanently covered. Do not farm task C, delay variants or duplicate lifecycle encodings merely to extend that phase.

## Selected milestone — bounded two-entry runnable queue

The next architecture boundary is scheduler selection rather than another trigger or lifecycle byte. Reuse the integrated A/B address spaces, supervisor-owned task-context page, context-switch scheduler and external wake path, but insert one guest-owned bounded queue that decides which runnable task is eligible to enter the existing scheduler.

This is deliberately a two-entry executable queue with explicit selection invariants, not a generic runqueue, priority/fairness policy, dynamic process table or second context-switch implementation.

Acceptance contract:

- preserve exact merged-green base `d656ea0370828c6c47ea39c7a3b83d058b4b3da1`, including #118 cooperative switching, #119 timer preemption, #120 block/wakeup lifecycle, Rust 1.74 shipped-target MSRV, ordinary CI and every permanent hosted-KVM workflow;
- retain the exact two address spaces, supervisor scheduler/context ABI, physical stack markers, CR3 ownership, R12 ownership and saved RIP contract from #118–#120;
- store bounded queue metadata in the existing supervisor-only task-context page: task A/B run states, queue entries `[A,B]`, head, selected task and cumulative blocked-entry skip count;
- reject duplicate task IDs in the two-entry model and hard-fail selection when neither entry is Runnable;
- task A must first transition to Blocked; with queue head `0`, the selector must inspect A, skip it because Blocked, inspect B, select B, advance the bounded head back to `0`, increment cumulative skip count exactly once and emit selector byte `1`;
- before transferring control to the existing scheduler, selecting B must verify the currently-running address space is A (`CR3=0x1000`); selecting A must verify the currently-running address space is B (`CR3=0xb000`); mismatch is a hard failure;
- queue selection is only eligibility/dispatch policy: the already-integrated scheduler bytes at the existing handler remain the sole implementation that saves contexts, switches CR3/stacks and returns with `iretq`;
- task B must arm the existing one-shot external wake path with IF clear; the GSI0/vector `0x40` handler changes A back to Runnable and returns to B;
- after wakeup the queue must begin again at head `0`, select now-Runnable A immediately, advance head to `1`, leave cumulative skip count at exactly `1`, and emit selector byte `0`;
- exact debug proof is `K1APW0BRD`: `K` blocks A, `1` proves blocked-A skip/B selection, `A` is the existing scheduler handoff from A, `P` arms external wake, `W` wakes A, `0` proves A selection, `B` is the existing scheduler handoff from B, `R` observes restored A, and `D` is completion;
- host-side direct queue snapshots must prove after the first selection: entries A/B, head `0`, selected B, skip count `1`, A Blocked and B Runnable; after the second selection: entries A/B, head `1`, selected A, skip count still `1`, A/B both Runnable;
- PIC/LAPIC state must retain GSI0→vector `0x40`, software-enabled SPIV and unmasked ExtINT LINT0 semantics; the arm observation must retain architectural RFLAGS bit 1 with IF clear;
- task A context remains `(CR3=0x1000, RIP=0x11011, RSP=0x1fcff0, RFLAGS=0x202, R12=0x1111, save_count=1)`;
- task B context remains `(CR3=0xb000, RIP=0x11013, RSP=0x1fcfd0, RFLAGS=0x202, R12=0x2223, save_count=1)`;
- terminal/final state must restore A with `CR3=0x1000`, user RIP `0x11013`, RSP `0x1fcff0`, R12 `0x1111`, physical stack markers `a`/`b`, and supervisor-only task-context PTEs backed by physical `0x30000`;
- KVM-aware integration and a permanent hosted-KVM `task-runnable-queue` workflow must independently hard-check both queue snapshots, controller state, arm flags, both contexts/save counts, terminal/final state, stack markers, context PTEs and exact `K1APW0BRD` proof;
- the permanent workflow must build the runnable-queue executable outside the 30-second execution timeout, then apply the timeout only to the actual hosted-KVM run so compile time cannot masquerade as an execution watchdog failure;
- formatter, Clippy, MSRV, queue encoding, selection, context switching, page-table ownership, proof, watchdog or hosted-KVM failures remain hard failures and must not be skipped, retried into success, or hidden by changing expected values.

Implementation is in progress on `milestone/bounded-runnable-queue` through PR #121. The queue runtime, executable, KVM-aware integration, permanent hosted-KVM workflow and roadmap synchronization are one coherent slice; only the exact final candidate with every applicable check green may be integrated.

## Scope boundary

This milestone deliberately does **not** add:

- task C, dynamic task/process allocation, an unbounded or generic runqueue, priorities, fairness, timeslices or periodic scheduling;
- multiple simultaneously blocked tasks, generic sleep/wait queues, wait-channel ownership, wake-one/wake-all semantics or signal delivery;
- cross-vCPU task migration, per-process TSS ownership or a claim that the bounded saved fields form a complete x86-64 task ABI;
- FPU/XSAVE, debug-register, signal-frame or complete register-state switching;
- more address spaces, mmap, demand paging, copy-on-write, swapping, PCID/ASID or KPTI;
- migration-complete snapshot semantics, dirty-ring/manual-protect2 or multi-slot dirty tracking;
- new filesystem/device/syscall surfaces, PCI/virtio expansion, DMA/IOMMU or performance/latency claims.

## Promotion rule

After the bounded queue is integrated and exact merged-`main` ordinary CI plus every permanent hosted-KVM workflow are green, seal this two-entry selector rather than adding task C, more fixed queue entries, duplicate selector bytes or fairness micro-policies.

The next architecture audit should promote to a materially broader lifecycle/state capability only when it can be demonstrated end-to-end. Strong candidates include composing task lifecycle/queue state with dirty or incremental-state capture, a bounded wait-channel ownership model that adds real wake semantics, or another higher-order execution boundary. Generic priorities, periodic scheduling, SMP task migration and process creation remain separate frontiers and require executable evidence rather than API scaffolding.
