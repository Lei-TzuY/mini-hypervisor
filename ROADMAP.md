# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `459dad4a88d6947819f41298cade474568688368` through PR #124 (`Keep KVM execution timeouts independent of cold builds`). PR #123 (`Capture scheduler wait state with KVM dirty logging`) is integrated immediately before it at `99193a4a352ef7fa414def3ecdb6d7714e46aae2`. Exact merged-main validation for #124 completed without observed failure across ordinary CI and all permanent hosted-KVM workflows for that exact commit.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; two isolated ring3 address spaces; opt-in slot-0 dirty-page tracking; bounded guest-owned A→B→A task context switching; host-timer task preemption; one bounded Runnable→Blocked→Runnable lifecycle with external wakeup; a two-entry guest-owned runnable selector; one bounded wait-channel owner with wrong-channel rejection and correct-channel external wake ownership; and incremental K/X/W/D dirty capture of the co-located scheduler/wait/task context page.

PR #118 seals cooperative two-task context switching. PR #119 seals host-timer preemption. PR #120 seals external Blocked→Runnable wakeup. PR #121 seals the bounded `[A,B]` runnable selector. PR #122 seals the fixed one-owner/one-waiter wait-channel proof. PR #123 seals the incremental dirty-capture composition: the exact #122 proof `K1AXPW0BRD` is preserved, slot-0 dirty logging is opt-in, every K/X/W/D primary harvest contains physical page `0x30000` (slot-0 page index 48), each capture is bound to its exact wait snapshot, and every immediate no-reentry residual harvest is zero. PR #124 then hardens permanent KVM workflows so compilation occurs outside the unchanged execution timeout rather than letting cold builds consume proof time.

The fixed wait-owner and K/X/W/D dirty-capture phases are sealed. Do not farm more barrier letters, alternate fixed channel values or duplicate bitmap fixtures.

## Selected milestone — bounded VCPU + guest-page checkpoint restore

The next architecture boundary is restore, not another dirty-log observation. Reuse the existing composite `VcpuStateSnapshot` (general registers, special registers and selected MSRs) and bind it to one exact 4 KiB guest page in a bounded checkpoint object. The executable proof must demonstrate real divergence, exact restore, fresh verification and resumed guest execution on hosted KVM before this primitive is composed back into the scheduler/wait page-48 state.

This is deliberately a one-VCPU/one-page restore primitive. It is not migration-complete snapshotting, device/controller replay, serialization, multi-page checkpointing or cross-host portability.

Acceptance contract:

- preserve exact merged-green base `459dad4a88d6947819f41298cade474568688368`, Rust 1.74 shipped-target MSRV, ordinary CI and every existing permanent hosted-KVM workflow;
- reuse `VcpuStateSnapshot` and its existing restore ordering/verification rather than introduce a parallel CPU-state model;
- checkpoint exactly one 4 KiB-aligned guest page plus one complete VCPU snapshot; reject a misaligned page rather than silently rounding;
- use GPA `0x30000`, deliberately matching the scheduler/wait context-page location selected by PR #123 without yet claiming full scheduler checkpoint composition;
- deterministic long-mode guest writes marker `0x5a` to GPA `0x30000`, reaches a non-serviceable HLT checkpoint boundary at RIP `0x10009`, and has no in-flight PIO/MMIO when state is captured;
- after capture, deliberately overwrite the entire page with `0xa5` and reinitialize the VCPU to a different but valid long-mode entry `0x12000` and stack `0x1fe000`; fresh comparison must independently report both page mismatch and VCPU-state mismatch before restore is attempted. Do not rely on a long-mode→real-mode `KVM_SET_SREGS` transition merely to manufacture divergence;
- restore the exact page bytes and restore the VCPU through the existing special-register → general-register → MSR dependency order, then perform fresh page readback and fresh VCPU capture; both comparisons must be exact;
- after restore, require marker `0x5a`, resume from the post-HLT checkpoint state, emit exact debug-port proof `R`, and terminate at a second HLT with RIP `0x1000e` and architectural RFLAGS bit 1;
- expose an executable and KVM-aware integration test that independently validate capture boundary, deliberate divergence, page/VCPU restore, one exact byte-wide debug-port exit and terminal boundary;
- add an independent permanent hosted-KVM workflow that builds `task-checkpoint-restore` outside its unchanged 30-second KVM execution timeout, requires `/dev/kvm`, and hard-checks both mismatch evidence and exact restore/resume evidence;
- formatter, Clippy, MSRV, snapshot capture/restore, page read/write, deliberate mismatch, fresh verification, proof or hosted-KVM failures remain hard failures and must not be skipped, retried into success or hidden by changed expected values.

Implementation is in progress on `milestone/task-wait-checkpoint-restore` through draft PR #125. The reusable checkpoint object, deterministic executable, KVM-aware integration, permanent hosted-KVM proof and roadmap synchronization form one coherent vertical slice; only an exact final candidate with every applicable check green may be integrated.

## Scope boundary

This milestone deliberately does **not** add:

- migration-complete or crash-consistent whole-VM snapshots, serialization formats, snapshot files, cross-host compatibility or replay logs;
- irqchip/LAPIC/PIC, irqfd/eventfd/ioeventfd, PCI/virtio or arbitrary device-state checkpoint/restore;
- multi-page or multi-slot restore, dirty-ring/manual-protect2, pre-copy/post-copy policy or automatic dirty-page selection;
- vCPU execution replay from an arbitrary in-flight PIO/MMIO exit; capture is intentionally at a stopped HLT boundary;
- a second waiter, multi-waiter/wake-one semantics, generic priorities, task/process allocation, SMP task migration or FPU/XSAVE/debug-register switching;
- demand paging, copy-on-write, swapping, DMA/IOMMU or performance/latency claims.

## Promotion rule

After the bounded one-page + VCPU checkpoint restore is integrated and exact merged-`main` ordinary CI plus every permanent hosted-KVM workflow are green, seal this generic restore primitive rather than adding a second arbitrary page fixture.

The next coherent milestone should bind the already integrated PR #123 scheduler/wait ownership state on physical page `0x30000` to this checkpoint primitive: capture a specific scheduler/wait barrier, deliberately mutate the typed task/wait ownership plus VCPU/page state, restore it, and require fresh ownership/context verification before any resumed execution. Device/controller-state checkpointing, multi-page migration and multi-waiter synchronization remain separate architectural frontiers.
