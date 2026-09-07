# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `1e597bf48ec95cc798cdcda9b859a4da6d8d3d2f` through PR #116 (`Switch between isolated ring3 address spaces`). Exact merged-main ordinary CI and every triggered permanent hosted-KVM workflow have converged: no failed, queued, in-progress or cancelled workflow remains for that exact commit.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall-number dispatcher; fault-safe copyin/copyout/usercopy with bounded cross-page partial progress; and two isolated ring3 address spaces with distinct CR3/page-table ownership and physical backing.

PR #116 seals the fixed A/B address-space proof. The same user code/data/stack virtual addresses are backed by different physical pages, the guest switches A→B→A through a shared CPL0 handler, CR3 observations are `[0x1000, 0xb000, 0x1000]`, physical data remains isolated, and exact proof `ABAD` completes at the validated CPL0 terminal path. Do not farm a third fixed address space, more hard-coded switch vectors or duplicate page-table variants.

## Selected milestone — bounded guest-owned task context switching

The next architecture boundary is execution-context ownership. Reuse the two integrated A/B address spaces, but add explicit per-task saved user execution state and a real guest-owned dispatch transition. A shared supervisor-only vector `0x80` handler must save the interrupted task context, select the other fixed task from the active CR3, switch address spaces, restore the next user frame/register state and return with `iretq`. Task A must later resume with its own register and stack state intact.

This remains a deliberately bounded cooperative two-task proof. It is not a general process object model, runqueue, timer-preemptive scheduler or claim that every architectural register is part of a stable task ABI.

Acceptance contract:

- preserve exact merged-green base `1e597bf48ec95cc798cdcda9b859a4da6d8d3d2f`, Rust 1.74 shipped-target MSRV, ordinary CI and every permanent hosted-KVM workflow green on PR #116 merged main;
- reuse address-space A at `CR3=0x1000` and address-space B at `CR3=0xb000`, including the already-validated distinct user code/stack physical backing and shared supervisor mappings;
- reserve one shared supervisor-only task-context page at physical `0x30000`; both roots must map it P/W with U clear;
- task A owns initial user RSP `0x1fcff0` and representative register `R12=0x1111`; task B owns initial user RSP `0x1fcfd0` and initial `R12=0x2222`;
- task A writes physical stack marker `a`, invokes vector `0x80`, and must save context `(CR3=0x1000, RIP=0x11011, RSP=0x1fcff0, RFLAGS=0x202, R12=0x1111, save_count=1)`;
- task B must start with restored `R12=0x2222`, write its independently backed stack marker `b`, increment R12 to `0x2223`, invoke the same scheduler vector, and save `(CR3=0xb000, RIP=0x11013, RSP=0x1fcfd0, RFLAGS=0x202, R12=0x2223, save_count=1)`;
- the CPL0 scheduler handler must choose A/B from the active CR3, fail closed on an unexpected root or representative-register value, save the current interrupted RIP/RSP/RFLAGS/R12 and CR3, rewrite a complete user `iretq` frame from the next context, restore next-task R12, switch CR3 and `iretq`; no host-side register or CR3 rewrite may substitute for the guest scheduler;
- after B→A, task A must reach terminal vector `0x81` with CR3 `0x1000`, user RIP `0x11013`, RSP `0x1fcff0` and R12 `0x1111`; final vCPU CR3/R12 must retain those A values;
- physical stack markers must be exactly `a` for A and `b` for B, proving the same user stack virtual page resolves to each task's independent backing while context state is restored;
- exact debug proof is `ABRD`: scheduler first observes A, scheduler then observes B, the terminal handler proves restored A state with `R`, then emits `D` before halting;
- terminal execution must halt in CPL0 at RIP `0x13059` with architectural RFLAGS bit 1 set;
- KVM-aware integration must independently validate every context field, both save counts, both physical stack markers, both supervisor task-context PTEs, all four byte-wide debug-port exits, final CR3/R12 and terminal state;
- a permanent `task-context-switch` hosted-KVM workflow must run the standalone binary under a bounded timeout with mandatory `/dev/kvm` and hard-check the same context, PTE, stack-marker, proof and terminal invariants;
- formatter, Clippy, Rust 1.74 MSRV, page-table ownership, context encoding, scheduler dispatch, user return frame, proof or hosted-KVM failures remain hard failures and must not be hidden by changed expectations.

Implementation is in progress on `milestone/task-context-switch`. It must pass ordinary CI and the new permanent hosted-KVM proof on one exact candidate before integration.

## Scope boundary

This milestone deliberately does **not** add:

- more than two fixed tasks, dynamic task/process allocation, PID namespaces, fork/exec, process teardown or a general process model;
- a runqueue, priorities, fairness policy, timer preemption, blocking/wakeup primitives or arbitrary scheduler selection;
- a claim that the bounded saved fields form a complete x86-64 process ABI; FPU/XSAVE, debug registers, signal state and every general register are not promoted here;
- a third address space, dynamic virtual mappings, mmap, copy-on-write, demand paging, swapping, PCID/ASID or KPTI;
- multi-vCPU user-task migration, cross-vCPU scheduling, per-process TSS ownership or signals;
- new filesystem/device/syscall surfaces, PCI/virtio capability, DMA/IOMMU or performance claims.

## Promotion rule

After the bounded A→B→A task-context switch is integrated and exact merged-`main` ordinary CI plus every triggered permanent hosted-KVM workflow are green, seal the fixed cooperative two-task proof rather than adding task C or more saved-field clones.

The next architecture audit should promote only to a materially new scheduling/control boundary with executable evidence. A strong candidate is bounded timer-driven preemption that combines the already integrated asynchronous interrupt source with the saved task-context/address-space ownership, but only if the slice can prove an interrupt-triggered context switch and deterministic resume without weakening the existing interrupt or task invariants. Otherwise choose another higher-value runtime boundary rather than inflating the task fixture.
