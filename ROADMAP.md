# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `59c9f28a1fb955dba3902d742b00de5e4ef68c83` through PR #118 (`Switch between bounded guest task contexts`). Every workflow triggered for that exact main has completed successfully; there are no failed, cancelled, queued, or in-progress checks on the integrated SHA.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; two isolated ring3 address spaces; opt-in slot-0 dirty-page tracking; and bounded guest-owned A→B→A task context switching.

PR #117 seals the basic dirty-log observability primitive. Slot 0 can opt into `KVM_MEM_LOG_DIRTY_PAGES`; the exact four-page fixture reports first bitmap `0b1010`, readback bytes `[A,B]`, and an immediate second harvest of zero without vCPU re-entry. This remains an observability/incremental-state prerequisite, not a migration-complete snapshot claim.

PR #118 seals the fixed cooperative two-task context-switch proof. Task A (`CR3=0x1000`, `R12=0x1111`, user RSP `0x1fcff0`) and task B (`CR3=0xb000`, initial `R12=0x2222`, user RSP `0x1fcfd0`) share supervisor scheduler/context state while retaining isolated user mappings. The guest-owned vector `0x80` scheduler saves/restores bounded user context, switches CR3, returns with `iretq`, and proves A→B→A with exact `ABRD`, independent physical stack markers, exact context save counts, final A CR3/R12 and supervisor-only context PTEs. Do not farm task C, more fixed cooperative vectors, or additional saved-field clones merely to extend this phase.

## Selected milestone — bounded timer-driven task preemption

The next architecture boundary is preemption ownership. Reuse the integrated A/B address spaces, task-context page and guest scheduler, but replace task A's cooperative scheduler trap with one host-driven timer interrupt. Task B remains cooperative so the proof can demonstrate that the timer-preempted A context was genuinely saved, B ran, and A later resumed from the interrupted state.

This is deliberately one bounded preemption path, not a general preemptive scheduler, runqueue, periodic scheduling policy or complete process ABI.

Acceptance contract:

- preserve exact merged-green base `59c9f28a1fb955dba3902d742b00de5e4ef68c83`, including the #118 task-context proof, #117 dirty-log behavior, Rust 1.74 shipped-target MSRV, ordinary CI and every permanent hosted-KVM workflow;
- keep the exact #118 A/B address spaces, shared supervisor scheduler state, task-context page, physical stack markers and bounded saved-context ABI;
- task A must not cooperatively invoke vector `0x80`; ring3 arms preemption through DPL3 vector `0x7f`;
- the arm handler must emit `P` while IF is clear and execute adjacent `sti; hlt`, using the x86 STI interrupt shadow as the race-safe handoff to a pending or later timer edge;
- a host worker may own only a duplicated VM IRQ-line fd, delay one bounded one-shot event and pulse GSI0; it must not own guest RAM, rewrite CR3/register state or substitute host scheduling for the guest scheduler;
- the in-kernel PIC must remain remapped to vectors `0x40..0x47`, only IRQ0 is unmasked, and LAPIC SPIV/LINT0 must retain the integrated software-enabled/unmasked ExtINT state;
- vector `0x40` must enter a CPL0 timer wrapper, send master-PIC EOI, discard only the nested CPL0 timer frame, and transfer control into the already-integrated guest scheduler;
- the scheduler must save preempted task A, dispatch B, accept B's existing cooperative return, restore A's original user context and resume it without host-side register/CR3 rewriting;
- exact debug proof is `PABRD`: `P` arm barrier, `A` timer-preempted task-A scheduler entry, `B` task-B scheduler entry, `R` restored-A terminal observation, `D` completion;
- exact task A context remains `(CR3=0x1000, RIP=0x11011, RSP=0x1fcff0, RFLAGS=0x202, R12=0x1111, save_count=1)`;
- exact task B context remains `(CR3=0xb000, RIP=0x11013, RSP=0x1fcfd0, RFLAGS=0x202, R12=0x2223, save_count=1)`;
- terminal/final state must restore A with `CR3=0x1000`, user RIP `0x11013`, RSP `0x1fcff0`, R12 `0x1111`, physical stack markers `a`/`b`, and supervisor-only task-context PTEs backed by physical `0x30000`;
- the arm observation must have architectural RFLAGS bit 1 set and IF clear; watchdog fallback may exist only to prevent a broken timer from wedging KVM and any watchdog intervention is a hard failure;
- KVM-aware integration and a permanent hosted-KVM `timer-task-preemption` workflow must hard-check GSI0/vector0x40, LAPIC state, arm flags, both contexts/save counts, terminal/final state, stack markers, context PTEs and exact `PABRD` proof;
- formatter, Clippy, MSRV, context encoding, timer-frame handling, scheduler dispatch, page-table ownership, proof or hosted-KVM failures remain hard failures and must not be skipped, retried into success, or hidden by changing expected values.

Implementation is in progress on `milestone/timer-task-preemption`. The current executable candidate has already demonstrated the intended hosted-KVM path, but any documentation/status synchronization changes the exact head and therefore requires a fresh complete workflow convergence before integration.

## Scope boundary

This milestone deliberately does **not** add:

- task C, dynamic task/process allocation, a runqueue, priorities, fairness, timeslices or periodic scheduling;
- a general preemptive scheduler, blocking/wakeup primitives, cross-vCPU task migration or per-process TSS ownership;
- a claim that the bounded saved fields form a complete x86-64 task ABI; FPU/XSAVE, debug registers, signals and complete register state remain outside this slice;
- more address spaces, mmap, demand paging, copy-on-write, swapping, PCID/ASID or KPTI;
- migration-complete snapshot semantics, dirty-ring/manual-protect2 or multi-slot dirty tracking;
- new filesystem/device/syscall surfaces, PCI/virtio capability, DMA/IOMMU or performance/latency claims.

## Promotion rule

After timer-preempted A→B→A is integrated and exact merged-`main` ordinary CI plus every permanent hosted-KVM workflow are green, seal the fixed one-shot preemption proof rather than adding more fixed timer delays, task C or scheduler-vector variants.

The next architecture audit should promote to a materially different scheduling or lifecycle capability. Strong candidates include a bounded runnable/blocked task lifecycle with a real wakeup source, composition of task-owned state with dirty/incremental-state capture, or another higher-order execution boundary that can be demonstrated end-to-end. A generic runqueue, periodic scheduler, fairness policy or migration claim must not be introduced without executable invariants and hosted-KVM evidence.
