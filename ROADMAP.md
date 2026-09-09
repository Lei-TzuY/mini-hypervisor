# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `e6b993e5d99963e9c0683dab0655578455ec7526` through PR #122 (`Add bounded wait-channel wake ownership`). Exact merged-main validation completed successfully across ordinary CI and all 30 permanent hosted-KVM workflows observed for that commit.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; two isolated ring3 address spaces; opt-in slot-0 dirty-page tracking; bounded guest-owned A→B→A task context switching; host-timer task preemption; one bounded Runnable→Blocked→Runnable lifecycle with external wakeup; a two-entry guest-owned runnable selector; and one bounded wait-channel owner with wrong-channel rejection and correct-channel external wake ownership.

PR #118 seals cooperative two-task context switching. PR #119 seals host-timer preemption. PR #120 seals external Blocked→Runnable wakeup. PR #121 seals the bounded `[A,B]` runnable selector. PR #122 seals the fixed one-owner/one-waiter wait-channel proof: task A owns channel `0x11` while blocked, wrong channel `0x22` is rejected without releasing ownership, the correct external GSI0/vector `0x40` wake makes A Runnable, and the queue reselects A. Exact hosted-KVM proof is `K1AXPW0BRD`, with queue snapshots, wait snapshots, A/B saved contexts, CR3/R12 ownership, physical stack markers and supervisor-only context PTEs permanently covered.

That wait-channel ownership phase is sealed. Do not farm channel `0x12`, task C, more fixed mismatch values or a second identical waiter merely to extend the phase number.

## Selected milestone — incremental dirty capture of scheduler/wait state

The next architecture boundary is state capture rather than another scheduler trigger. Reuse the exact #122 guest, wait-channel ownership model, two-entry queue, external wake path and supervisor-only task-context page, but register slot 0 with the existing KVM dirty-log primitive and prove that committed scheduler/wait mutations become incrementally observable and clearable.

This is deliberately a bounded dirty-capture composition over the existing single memslot. It is not migration-complete snapshotting, dirty-ring/manual-protect2, multi-slot tracking or restore.

Acceptance contract:

- preserve exact merged-green base `e6b993e5d99963e9c0683dab0655578455ec7526`, Rust 1.74 shipped-target MSRV, ordinary CI and every existing permanent hosted-KVM workflow;
- preserve the exact #122 machine code, wait-channel model, queue selector, task contexts, CR3/R12 ownership, stack markers, supervisor-only context PTEs, GSI0/vector `0x40` path and exact debug proof `K1AXPW0BRD`;
- reuse the existing slot-0 `KVM_GET_DIRTY_LOG` implementation and `KVM_MEM_LOG_DIRTY_PAGES` registration path rather than creating a second dirty-log UAPI or making dirty tracking globally mandatory;
- expose only the minimal crate-private dirty-log session boundary required by task execution; no new public generic dirty-log framework is introduced;
- enable dirty logging only for the new wait-state capture runner; the integrated `run_bounded_wait_channel_guest` must retain its ordinary memslot behavior and source-level compatibility;
- finish guest-image, page-table, task-context, queue/wait metadata and vCPU/controller initialization before measurement; drain setup dirtiness once, then require an immediate second pre-execution harvest to be entirely zero;
- retain the existing four host-visible commit barriers `K`, `X`, `W` and `D` as capture boundaries: `K` follows block/owner mutation, `X` follows wrong-wake metadata mutation, `W` follows correct wake ownership release, and `D` follows final scheduler/task terminal-state updates;
- at every K/X/W/D capture, the dirty bitmap must include the supervisor task-context page at physical `0x30000` (slot-0 page index 48), because task contexts, queue/wait metadata and terminal observation are deliberately co-located on that page;
- do not predeclare the rest of each dirty bitmap as an exact exclusive set: stack, execution or implementation-specific dirty bits may also be observed. Preserve and expose the complete bitmap as evidence and hard-check only architectural claims that the guest actually requires;
- immediately after every primary K/X/W/D harvest, without re-entering the guest, perform a second harvest and require every word to be zero. A residual bit is a hard failure and proves the interval cannot be used as an incremental capture boundary;
- bind each primary bitmap to the wait snapshot observed at the same barrier: K=`Blocked/owner 0x11/mismatches 0/wakes 0/last 0`, X=`Blocked/owner 0x11/mismatches 1/wakes 0/last 0x22`, W and D=`Runnable/owner 0/mismatches 1/wakes 1/last 0x11`;
- the final result must still pass every existing #122 queue/context/terminal validator and retain exact proof bytes `[75, 49, 65, 88, 80, 87, 48, 66, 82, 68]`;
- provide a deterministic executable that prints all four complete dirty bitmaps, stage identity, context-page evidence and bound wait snapshot, plus a KVM-aware integration test;
- add an independent permanent hosted-KVM `task-wait-dirty-capture` workflow. It must build outside the execution timeout, require `/dev/kvm`, hard-check `K1AXPW0BRD`, all four stages, page-48 dirtiness, bound wait snapshots, controller state and arm flags, and may not skip KVM unavailability;
- formatter, Clippy, MSRV, dirty-log baseline/clear semantics, context-page evidence, wait ownership, queue selection, context switching, proof, watchdog or hosted-KVM failures remain hard failures and must not be skipped, retried into success or hidden by changed expected values.

Implementation is in progress on `milestone/task-wait-dirty-capture`. The reusable crate-private dirty-log session, shared #122 execution path, executable, KVM-aware integration, permanent hosted-KVM workflow and roadmap synchronization form one coherent vertical slice; only an exact final candidate with every applicable check green may be integrated.

## Scope boundary

This milestone deliberately does **not** add:

- a second fixed waiter, task C, dynamic task/process allocation, generic runqueues, priorities, fairness or periodic scheduling;
- multiple waiters per channel, wait queues, wake-one/wake-all policy, condition variables, futexes, signals or cancellation;
- migration-complete snapshots, restore/replay, device-state capture, multi-slot dirty tracking, dirty ring, manual dirty-log protect2 or post-copy/pre-copy migration policy;
- cross-vCPU task migration, SMP scheduler ownership, FPU/XSAVE/debug-register switching or a complete x86-64 task ABI;
- demand paging, copy-on-write, swapping, PCID/ASID, KPTI, new filesystem/device/syscall surfaces, PCI/virtio expansion, DMA/IOMMU or performance/latency claims.

## Promotion rule

After scheduler/wait dirty capture is integrated and exact merged-`main` ordinary CI plus every permanent hosted-KVM workflow are green, seal the fixed K/X/W/D incremental-capture proof rather than adding more barrier letters or duplicate bitmap fixtures.

The next architecture audit should promote to a materially broader state-management or synchronization boundary. Strong candidates are a bounded checkpoint object that combines dirty guest pages with the already typed CPU/task ownership needed for a verifiable restore, or a genuinely bounded multi-waiter/wake-one model if it adds real ordering/ownership semantics rather than fixed-count repetition. Generic priorities, SMP task migration, process creation and unbounded synchronization remain separate frontiers.
