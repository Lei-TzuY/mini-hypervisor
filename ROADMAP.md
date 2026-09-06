# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `b119c069a5578b624a709253115ef745dec89a51` through PR #107 (`Shoot down a remote vCPU TLB entry`). The repository integrates the Phase 73 foundation, x86-64/ELF64 execution, userspace and virtual MMIO, controller-backed interrupts, direct and irqfd/eventfd asynchronous delivery, PCI/virtio-rng/virtio-blk paths, and a bounded two-vCPU SMP control/data plane.

The SMP path includes guest-owned INIT/SIPI AP startup, AP real-mode-to-long-mode transition, guest-originated xAPIC IPIs, shared mailbox work dispatch, targeted MSI delivery, an AP-owned local LAPIC timer, and one fixed cross-vCPU TLB shootdown. PR #107 proves that the AP establishes one shared alias translation, the BSP mutates the shared PTE and sends vector `0x54`, the AP handler executes `invlpg`, acknowledges completion, and then observes the replacement page through the same VA with exact Accessed/Dirty semantics.

Exact merged-main ordinary CI and every permanent hosted-KVM workflow for `b119c069a5578b624a709253115ef745dec89a51` are green. The fixed one-target shootdown is sealed. Do not farm extra target pages, vectors, AP clones, or stale-read timing variants merely to extend the phase number.

## Selected milestone — bounded x86-64 ring3/TSS privilege transition

The next architecture boundary is privilege separation. The guest must own the descriptor state that makes a hardware CPL3→CPL0 transition possible: guest-backed GDT, IDT, and 64-bit TSS; user/supervisor page permissions; guest `ltr`; a five-word `iretq` entry frame; and DPL3 interrupt gates that force a TSS RSP0 stack switch.

This is deliberately one user context and one vCPU. It proves the privilege boundary, stack ownership, descriptor transition, and return path without claiming a process model, syscall ABI, per-process CR3, or general user fault recovery.

Acceptance contract:

- preserve ordinary CI, Rust 1.74 shipped-target MSRV, and every permanent hosted-KVM workflow already green on merged `main`;
- reuse the integrated long-mode CR0/CR4/EFER setup while explicitly installing guest GDT/IDT bases and limits for the privilege fixture;
- install ring0 code/data descriptors, ring3 code/data descriptors, and one 64-bit TSS descriptor at selector `0x28`;
- do not pre-load TR from userspace: guest code must execute `ltr 0x28`, and runtime readback must prove selector `0x28`, base `0x7000`, limit `0x67`, busy TSS type `0x0b`, and descriptor access byte transition to `0x8b`;
- map only the user code page, selector-observation page, and user stack page with the U/S bit; ring0 handlers and kernel stacks remain supervisor-only;
- enter CPL3 through an explicit five-word `iretq` frame with CS `0x23`, SS `0x1b`, RSP `0x1fd000`, RFLAGS `0x202`, and user RIP `0x11000`;
- vector `0x80` is a DPL3 interrupt gate to ring0 handler `0x12000`; hardware must switch to TSS RSP0 `0x1fe000`, the handler emits `K`, and `iretq` returns to ring3;
- ring3 must observe CS/SS as `0x23/0x1b` before and after that return, proving privilege restoration rather than only one-way entry;
- vector `0x81` is a second DPL3 interrupt gate to ring0 handler `0x13000`; it must traverse the same TSS ownership, emit `D`, and terminate in ring0;
- exact debug-port proof is `KD` across two byte-wide OUT exits followed by HLT at RIP `0x13005`;
- the first hardware privilege frame at `RSP0-40` must contain RIP `0x11025`, CS `0x23`, RFLAGS `0x202`, RSP `0x1fd000`, SS `0x1b`;
- terminal ring0 state must use RSP `0x1fdfd8`, CS `0x08`, architectural RFLAGS bit1 set and IF clear;
- KVM-aware integration must independently validate both port-I/O exits, selector observations, privilege frame, terminal ring0 state, TR state, TSS busy bit, and user/supervisor PTE semantics;
- deterministic regressions must validate GDT/TSS encoding, DPL3 IDT gates, user/supervisor PTE ownership, reserved-table separation, guest `ltr`, and the fixed privilege-frame layout;
- executable evidence must expose exact proof, selector observations, privilege frame, terminal ring0 state, TR selector/base/limit/type, TSS access byte, page-permission PTEs, and terminal HLT report;
- permanent workflow `Strict KVM ring3 TSS privilege transition` must run independently on hosted KVM with a bounded timeout and require the executable evidence above; it may not skip `/dev/kvm` or convert a privilege failure into success;
- formatter, Clippy, MSRV, descriptor encoding, page permissions, `ltr`, CPL transition, stack switch, `iretq`, proof, or architectural-state failures remain hard failures and must not be hidden by changed expectations, retries-to-success, or weakened permanent gates.

The implementation is in progress on `milestone/ring3-tss-privilege-transition` / PR #108. The pre-governance candidate `0bfc4734b16fcf3e2edbdbd4815a83caa6e9118e` passed the existing ordinary and permanent workflow set, including the KVM-aware ring3 integration test. ROADMAP synchronization and the dedicated permanent ring3/TSS workflow are now part of this same coherent slice. No capability is considered integrated until the final exact candidate passes all applicable checks, remains current with `main`, completes the normal review/merge audit, and the merged-main permanent ring3 workflow succeeds.

## Scope boundary

This milestone deliberately does **not** add:

- `SYSCALL/SYSRET`, SYSENTER/SYSEXIT, a syscall table, user ABI, or kernel service layer;
- TSS I/O-bitmap permissions, user-mode port-I/O enablement, IOPL policy, or per-task TSS switching;
- user #PF/#GP recovery, exception-to-user signal delivery, demand paging, copyin/copyout, or fault-safe user pointers;
- process CR3 switching, per-process address spaces, PCID, ASIDs, scheduler/context switching, multiple user tasks, or executable loading into ring3;
- SMP privilege transitions, per-vCPU TSS ownership across multiple CPUs, cross-vCPU user tasks, or migration of privilege state;
- new timer, MMIO, PCI/virtio, storage, DMA/IOMMU, performance, latency, persistence, or durability claims.

## Promotion rule

After the bounded ring3/TSS transition is integrated and exact merged-`main` ordinary CI plus every permanent workflow are green, seal the fixed one-user-context proof rather than adding extra vectors, selector variants, or additional identical ring3 tasks.

The next architecture audit should select a genuinely higher-order boundary. Strong candidates are a minimal `SYSCALL/SYSRET` system-call ABI that reuses the integrated privilege ownership, or a reusable per-vCPU privilege/TSS model only if it introduces real multi-vCPU execution evidence. More fixed DPL3 trap variants are not a promotion.
