# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `41e5c46730d742be3915879cec46dfefe92dc078` through PR #119 (`Preempt bounded guest tasks from a host timer`). The exact merged main is the green integration base for the next slice.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; two isolated ring3 address spaces; opt-in slot-0 dirty-page tracking; bounded guest-owned A→B→A task context switching; and one host-timer-driven task-preemption path.

PR #118 seals the fixed cooperative two-task context switch. PR #119 seals the next preemption boundary: task A no longer cooperatively yields first; a one-shot host timer reaches the guest through GSI0/vector `0x40`, the guest-owned scheduler saves the interrupted A context, dispatches B, accepts B's cooperative return and restores A. Exact hosted-KVM proof `PABRD`, A/B context save counts, CR3/R12 ownership, physical stack markers and supervisor-only context PTEs are permanently covered. Do not farm more fixed timer delays, task C or scheduler-vector clones merely to extend that phase.

## Selected milestone — blocked task lifecycle with external wakeup

The next architecture boundary is task lifecycle state rather than another scheduling trigger. Reuse the integrated A/B address spaces, supervisor-owned task-context page, scheduler bytes and external timer delivery, then prove one bounded `Runnable → Blocked → Runnable` transition for task A while task B runs during the blocked interval.

This is deliberately one executable block/wake path, not a generic runqueue, blocking queue, sleep API, wait channel, priority policy or process lifecycle model.

Acceptance contract:

- preserve exact merged-green base `41e5c46730d742be3915879cec46dfefe92dc078`, including #118 cooperative switching, #119 timer preemption, Rust 1.74 shipped-target MSRV, ordinary CI and every permanent hosted-KVM workflow;
- retain the exact two address spaces, shared supervisor scheduler/context ABI, physical stack markers, CR3 ownership, R12 ownership and saved RIP contract from #118/#119;
- keep task lifecycle state in the existing supervisor-only shared task-context page at GPA `0x300c0`; host observations must read that state directly rather than infer lifecycle solely from debug-port bytes;
- task A enters DPL3 vector `0x7e`; the CPL0 block wrapper stores `Blocked`, emits `K`, and jumps into the already-integrated scheduler without rewriting the user interrupt frame;
- the scheduler saves A and dispatches B using the existing bounded context-switch implementation rather than a duplicate scheduler path;
- task B enters DPL3 vector `0x7f`; the arm wrapper emits `P` with IF clear, executes adjacent `sti; hlt`, verifies shared state is `Runnable` after wakeup, then transfers into the existing scheduler;
- a one-shot host worker owns only a duplicated VM IRQ-line fd and pulses GSI0; it must not own guest RAM or rewrite guest registers/CR3; the watchdog exists solely to avoid an infinite KVM wait and any watchdog intervention is a hard failure;
- vector `0x40` stores shared state `Runnable`, emits `W`, sends master-PIC EOI and `iretq`s back to the arm wrapper;
- exact debug proof is `KAPWBRD`: `K` block transition, `A` scheduler entry from blocked A, `P` wake arm barrier, `W` external timer handler, `B` scheduler entry from B, `R` restored-A terminal observation, `D` completion;
- direct host state observations must prove `Blocked` immediately after `K`, `Runnable` immediately after `W`, and `Runnable` at final completion;
- the arm observation must have architectural RFLAGS bit 1 set and IF clear; PIC/LAPIC state must retain GSI0→vector `0x40`, software-enabled SPIV and unmasked ExtINT LINT0 semantics;
- task A context remains `(CR3=0x1000, RIP=0x11011, RSP=0x1fcff0, RFLAGS=0x202, R12=0x1111, save_count=1)`;
- task B context remains `(CR3=0xb000, RIP=0x11013, RSP=0x1fcfd0, RFLAGS=0x202, R12=0x2223, save_count=1)`;
- terminal/final state must restore A with `CR3=0x1000`, user RIP `0x11013`, RSP `0x1fcff0`, R12 `0x1111`, physical stack markers `a`/`b`, and supervisor-only task-context PTEs backed by physical `0x30000`;
- KVM-aware integration and a permanent hosted-KVM `task-block-wakeup` workflow must independently hard-check lifecycle states, controller state, arm flags, both contexts/save counts, terminal/final state, stack markers, context PTEs and exact `KAPWBRD` proof;
- formatter, Clippy, MSRV, state encoding, interrupt-frame handling, scheduler dispatch, page-table ownership, proof, watchdog or hosted-KVM failures remain hard failures and must not be skipped, retried into success, or hidden by changing expected values.

Implementation is in progress on `milestone/task-block-wakeup` through PR #120. The production runtime, executable, KVM-aware integration and permanent hosted-KVM workflow are part of one coherent slice; the exact docs-synchronized head must converge completely before integration.

## Scope boundary

This milestone deliberately does **not** add:

- task C, dynamic task/process allocation, a generic runqueue, priorities, fairness, timeslices or periodic scheduling;
- generic sleep/wait queues, multiple blocked tasks, wait-channel ownership, wake-one/wake-all semantics or signal delivery;
- cross-vCPU task migration, per-process TSS ownership or a claim that the bounded saved fields form a complete x86-64 task ABI;
- FPU/XSAVE, debug-register, signal-frame or complete register-state switching;
- more address spaces, mmap, demand paging, copy-on-write, swapping, PCID/ASID or KPTI;
- migration-complete snapshot semantics, dirty-ring/manual-protect2 or multi-slot dirty tracking;
- new filesystem/device/syscall surfaces, PCI/virtio expansion, DMA/IOMMU or performance/latency claims.

## Promotion rule

After the blocked→external-wake→resume lifecycle is integrated and exact merged-`main` ordinary CI plus every permanent hosted-KVM workflow are green, seal this fixed A/B lifecycle rather than adding task C, another fixed lifecycle byte, more delay variants or duplicate state encodings.

The next architecture audit should promote to a materially broader scheduler/lifecycle capability only when it can be demonstrated end-to-end. Strong candidates include a bounded two-entry runnable queue with explicit selection invariants, composition of task lifecycle state with dirty/incremental-state capture, or another higher-order execution boundary. Generic fairness, priorities, periodic scheduling and migration remain separate frontiers and require executable evidence rather than API scaffolding.
