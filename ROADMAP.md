# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `4f8b063e7749d729424f29d24fb3d93bc7507e13` through PR #121 (`Select runnable guest tasks from a bounded queue`). Exact merged-main validation completed successfully across ordinary CI and all 29 permanent hosted-KVM workflows observed for that commit.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; two isolated ring3 address spaces; opt-in slot-0 dirty-page tracking; bounded guest-owned A→B→A task context switching; host-timer task preemption; one bounded Runnable→Blocked→Runnable lifecycle with external wakeup; and a two-entry guest-owned runnable selector.

PR #118 seals fixed cooperative two-task context switching. PR #119 seals host-timer preemption. PR #120 seals the external Blocked→Runnable wake path. PR #121 seals the bounded `[A,B]` runnable selector: blocked A is skipped exactly once to select B, external wake makes A Runnable, and the next selection returns to A without an additional skip. Exact hosted-KVM proof `K1APW0BRD`, queue snapshots, A/B saved contexts, CR3/R12 ownership, physical stack markers and supervisor-only context PTEs are permanently covered. Do not farm task C, additional fixed queue entries, delay variants or fairness micro-policies merely to extend the phase.

## Selected milestone — bounded wait-channel ownership

The next architecture boundary is wake ownership rather than another scheduler trigger. Reuse the integrated A/B address spaces, supervisor-owned context page, bounded runnable selector, existing scheduler and external GSI0 wake path, but attach one explicit wait-channel owner to blocked task A and require wake attempts to match that owner before A becomes Runnable.

This is deliberately one bounded channel-owner lifecycle, not a generic sleep queue, channel hash table, wake-all API or multi-waiter scheduler.

Acceptance contract:

- preserve exact merged-green base `4f8b063e7749d729424f29d24fb3d93bc7507e13`, Rust 1.74 shipped-target MSRV, ordinary CI and every permanent hosted-KVM workflow;
- retain the exact two address spaces, existing scheduler bytes, queue selector, physical stack markers, CR3/R12 ownership and A/B saved RIP contract from #118–#121;
- store wait metadata in the existing supervisor-only task-context page without changing its user/supervisor page ownership: owner, mismatch count, successful wake count and last attempted channel;
- reserve channel `0` for no owner; task A blocks on fixed channel `0x11`, transitions to Blocked and records owner `0x11` before entering the existing queue selector;
- after `K`, host-visible wait state must be exactly `Blocked / owner=0x11 / mismatches=0 / wakes=0 / last=0`;
- the existing queue must skip blocked A and select B exactly as #121, retaining first snapshot `[A,B], head=0, selected=B, skips=1, A=Blocked, B=Runnable`;
- before arming the external wake, the existing B-side wrapper must attempt wrong channel `0x22`; the attempt must not change A to Runnable or release owner `0x11`, must increment mismatch count exactly once, record last attempt `0x22`, and emit `X`;
- after the mismatch, wait state must be exactly `Blocked / owner=0x11 / mismatches=1 / wakes=0 / last=0x22`;
- B then emits existing arm byte `P` with IF clear and uses the existing race-safe `sti; hlt` external-wake handoff;
- the external GSI0/vector `0x40` handler represents the correct channel `0x11`; it must verify ownership before waking, set A Runnable, clear owner to `0`, increment successful wake count exactly once, record last attempt `0x11`, emit `W`, issue PIC EOI and `iretq`;
- after the correct wake and at final completion, wait state must be exactly `Runnable / owner=0 / mismatches=1 / wakes=1 / last=0x11`;
- the existing queue must then select now-Runnable A without another skip, retaining second snapshot `[A,B], head=1, selected=A, skips=1, A=Runnable, B=Runnable`;
- exact debug proof is `K1AXPW0BRD`: `K` blocks A and claims channel `0x11`; `1` selects B after skipping A; `A` is the existing scheduler handoff; `X` proves the wrong-channel wake was rejected; `P` arms external wake; `W` is the correct-channel wake; `0` reselects A; `B` is the scheduler handoff from B; `R` observes restored A; `D` is completion;
- task A context remains `(CR3=0x1000, RIP=0x11011, RSP=0x1fcff0, RFLAGS=0x202, R12=0x1111, save_count=1)` and task B remains `(CR3=0xb000, RIP=0x11013, RSP=0x1fcfd0, RFLAGS=0x202, R12=0x2223, save_count=1)`;
- terminal/final state must restore A with `CR3=0x1000`, user RIP `0x11013`, RSP `0x1fcff0`, R12 `0x1111`, physical stack markers `a`/`b`, and supervisor-only context PTEs backed by physical `0x30000`;
- PIC/LAPIC state must retain GSI0→vector `0x40`, software-enabled SPIV and unmasked ExtINT LINT0; the `P` arm observation must retain architectural RFLAGS bit 1 with IF clear;
- a pure wait-channel model must reject channel zero, double blocking, wake without ownership and mismatched wake release; production snapshot validation must consume that model rather than leaving it test-only;
- KVM-aware integration and a permanent hosted-KVM `task-wait-channel` workflow must independently hard-check all wait snapshots, both queue snapshots, controller state, arm flags, both saved contexts, final ownership, stack markers, context PTEs and exact proof `[75, 49, 65, 88, 80, 87, 48, 66, 82, 68]`;
- the permanent workflow must build outside the 30-second execution timeout and apply the timeout only to the hosted-KVM run; KVM unavailability in the strict workflow is a hard failure;
- formatter, Clippy, MSRV, channel ownership, wrong-channel rejection, queue selection, context switching, page-table ownership, proof, watchdog or hosted-KVM failures remain hard failures and must not be skipped, retried into success, or hidden by changed expected values.

Implementation is in progress on `milestone/bounded-wait-channel`. The wait-channel runtime, executable, KVM-aware integration, permanent hosted-KVM workflow and roadmap synchronization are one coherent slice; only an exact final candidate with every applicable check green may be integrated.

## Scope boundary

This milestone deliberately does **not** add:

- task C, dynamic task/process allocation, a generic or unbounded runqueue, priorities, fairness, timeslices or periodic scheduling;
- multiple simultaneously blocked tasks, multiple waiters per channel, a channel registry/hash table, wait queues, wake-one/wake-all policy, condition variables, futexes, signals or cancellation;
- cross-vCPU task migration, per-process TSS ownership or a claim that the bounded saved fields form a complete x86-64 task ABI;
- FPU/XSAVE, debug-register, signal-frame or complete register-state switching;
- more address spaces, mmap, demand paging, copy-on-write, swapping, PCID/ASID or KPTI;
- migration-complete snapshot semantics, dirty-ring/manual-protect2 or multi-slot dirty tracking;
- new filesystem/device/syscall surfaces, PCI/virtio expansion, DMA/IOMMU or performance/latency claims.

## Promotion rule

After bounded wait-channel ownership is integrated and exact merged-`main` ordinary CI plus every permanent hosted-KVM workflow are green, seal this fixed one-owner/one-waiter channel proof rather than adding channel `0x12`, task C or duplicate mismatch vectors.

The next architecture audit should promote only to a materially broader state boundary with executable evidence. Strong candidates are composing scheduler/wait state with dirty or incremental-state capture, or extending wait semantics to a genuinely bounded multi-waiter/wake-one model if that adds real ownership and ordering behavior. Generic priorities, periodic scheduling, SMP task migration, process creation and unbounded synchronization remain separate frontiers.
