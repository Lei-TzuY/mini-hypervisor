# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `318cd9513e89978728382ae780149d5f3627f7f1` through PR #125 (`Restore a bounded VCPU and guest page checkpoint`). Exact merged-main validation for #125 completed successfully across ordinary CI and every permanent hosted-KVM workflow triggered for that exact commit.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; two isolated ring3 address spaces; opt-in slot-0 dirty-page tracking; bounded guest-owned A→B→A task context switching; host-timer task preemption; one bounded Runnable→Blocked→Runnable lifecycle with external wakeup; a two-entry guest-owned runnable selector; one bounded wait-channel owner with wrong-channel rejection and correct-channel external wake ownership; incremental K/X/W/D dirty capture of the co-located scheduler/wait/task context page; and one bounded checkpoint containing a full VCPU snapshot plus one exact 4 KiB guest page.

PR #118 seals cooperative two-task context switching. PR #119 seals host-timer preemption. PR #120 seals external Blocked→Runnable wakeup. PR #121 seals the bounded `[A,B]` runnable selector. PR #122 seals the fixed one-owner/one-waiter wait-channel proof. PR #123 seals incremental K/X/W/D dirty capture of physical page `0x30000`. PR #124 keeps strict KVM execution timeouts independent of cold compilation. PR #125 seals the reusable one-VCPU/one-page checkpoint primitive: capture at a non-serviceable boundary, prove deliberate VCPU/page divergence, restore through the existing dependency ordering, perform fresh exact verification, then resume execution on real KVM.

The generic one-page checkpoint primitive is sealed. Do not farm second arbitrary pages, extra marker values or duplicate capture/restore fixtures merely to extend the phase number.

## Selected milestone — scheduler wait ownership checkpoint composition

The next architecture boundary is composition of the generic #125 checkpoint with the already integrated scheduler/wait ownership state on physical page `0x30000`. The existing wait-channel execution remains the single scheduler path. Checkpoint mode must capture a proven non-serviceable boundary after the wrong-channel `X` output is committed but before wake-arm output `P` begins, deliberately corrupt both machine state and typed ownership, restore both, perform fresh exact verification, and only then resume the original wait/wakeup path.

This is deliberately one VCPU, one scheduler context page, one fixed wait owner and one runnable-queue snapshot. It is not device/controller checkpointing, migration-complete snapshotting, multi-page restore or multi-waiter synchronization.

Acceptance contract:

- preserve exact merged-green base `318cd9513e89978728382ae780149d5f3627f7f1`, Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow;
- preserve the integrated plain wait-channel proof `K1AXPW0BRD` and the existing K/X/W/D dirty-capture path unchanged;
- never capture or restore VCPU state while a PIO/MMIO service exit is in-flight;
- checkpoint mode uses a dedicated wake-arm byte sequence where `X` occupies offsets `0x39..0x3c`, a NOP is at `0x3d`, and `P` begins at `0x3e`;
- after observing serviceable `X`, enable KVM guest single-step and re-enter: KVM must complete the prior OUT and report typed `Debug` at RIP `0x1603d`, the NOP instruction start, with architectural RFLAGS bit 1; the NOP and `P` have not executed and the checkpoint execution result must contain no serviced port-I/O exit;
- reuse `BoundedVcpuPageCheckpoint` and the existing VCPU restore dependency ordering rather than create a second snapshot model;
- capture typed scheduler ownership at the Debug boundary: wait-channel snapshot, runnable-queue snapshot, task-A context and task-B context;
- deliberately mutate page-backed wait/queue ownership to a valid-but-wrong state and mutate the VCPU to a different valid state;
- fresh corruption comparison must independently prove page mismatch, VCPU mismatch, wait mismatch and queue mismatch while the task-A/task-B context entries that were not deliberately changed remain exact;
- exact restore must restore the captured page, restore VCPU state through the existing ordering, then perform fresh typed ownership reads; no guest resume is allowed until page, VCPU, wait, queue and both task-context comparisons are exact;
- after restore, continue the original `P→W→0→B→R→D` execution and preserve exact proof `K1AXPW0BRD`;
- KVM-aware integration must independently validate the Debug boundary, captured ownership, deliberate mismatch surface, exact restore surface and unchanged final wait proof;
- the permanent hosted-KVM gate must build `task-wait-checkpoint-composition` before the unchanged 30-second execution timeout and require exact `Debug@0x1603d`, ownership/corruption/restore evidence and proof bytes `[75, 49, 65, 88, 80, 87, 48, 66, 82, 68]`;
- legacy permanent KVM workflows touched by the new typed Debug surface must likewise keep cold compilation outside their unchanged execution timeout; this is CI execution isolation, not an expanded KVM timeout;
- formatter, Clippy, MSRV, Debug classification, checkpoint capture/restore, typed ownership readback, deliberate mismatch, fresh verification, proof or hosted-KVM failures remain hard failures and must not be skipped, retried into success or hidden by changed expected values.

Implementation is in progress on `milestone/task-wait-checkpoint-composition` through draft PR #126. The candidate has already demonstrated the corrected serviceable-exit completion boundary on hosted KVM: `Debug` at RIP `0x1603d`, blocked owner `0x11` with one wrong-channel mismatch, deliberate page/VCPU/wait/queue mismatch, exact page/VCPU/wait/queue/task-context restore, and unchanged `K1AXPW0BRD` execution. Only the final docs-synchronized exact candidate with every applicable workflow green may be integrated.

## Scope boundary

This milestone deliberately does **not** add:

- irqchip/LAPIC/PIC, irqfd/eventfd/ioeventfd, PCI/virtio or arbitrary device/controller-state checkpoint/restore;
- multi-page or multi-slot checkpoints, automatic dirty-page selection, dirty-ring/manual-protect2, pre-copy/post-copy policy or replay logs;
- capture or replay from arbitrary in-flight PIO/MMIO exits;
- a second waiter, multi-waiter/wake-one synchronization, generic scheduler priorities, task/process allocation or SMP task migration;
- FPU/XSAVE/debug-register checkpointing, migration serialization/versioning, cross-host compatibility or crash-consistent whole-VM snapshots;
- demand paging, copy-on-write, swapping, DMA/IOMMU or performance/latency claims.

## Promotion rule

After scheduler wait ownership checkpoint composition is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this one-owner/one-page composition rather than adding alternate channel values or another fixed scheduler barrier.

The next architecture audit should choose a materially broader checkpoint frontier. Prefer either a coherent multi-page guest-state checkpoint selected by an explicit ownership set, or device/controller-state checkpoint composition only when capture/restore ordering and executable KVM evidence can be made explicit. Migration serialization/versioning, in-flight service-exit replay, multi-waiter synchronization and cross-host portability remain separate higher-order frontiers.