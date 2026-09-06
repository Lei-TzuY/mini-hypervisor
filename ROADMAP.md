# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `28dc682079c9d9ce7ff9fabf7dc2198178d4146f` through PR #105 (`Dispatch work through the SIPI-started AP`). The repository now integrates the Phase 73 foundation, x86-64/ELF64 execution, userspace and virtual MMIO, controller-backed interrupts, direct and irqfd/eventfd asynchronous delivery, PCI/virtio-rng/virtio-blk paths, and a bounded two-vCPU SMP control/data plane.

The SMP path now composes the previously separate pieces in one VM: vCPU1 begins `KVM_MP_STATE_UNINITIALIZED`, the guest BSP performs xAPIC INIT/SIPI vector `0x08`, the AP owns its real-mode-to-long-mode transition, the BSP publishes the fixed mailbox work item at GPA `0x9000`, and the guest-originated vector-`0x52` IPI notifies the running AP. The AP consumes the command with locked `XCHG`, computes `0x21→0x42`, publishes acknowledgement ownership, and both vCPUs terminate their acceptance paths at userspace-visible debug-port completion barriers rather than relying on an in-kernel-LAPIC HLT exit.

PR #105 and its exact merged-main ordinary CI plus permanent hosted-KVM workflows are green at this boundary. The fixed one-work-item SIPI/IPI/mailbox composition is sealed. Do not farm alternate payloads, vectors, APIC IDs, SIPI vectors, polling constants, or vCPU2/vCPU3 clones merely to extend the phase number.

## Selected milestone — SIPI-started AP owns a one-shot local LAPIC timer

The next boundary is per-vCPU local timer ownership. The AP must retain the integrated INIT/SIPI startup and guest-owned long-mode transition, then program and service its own xAPIC local timer instead of receiving the event from the BSP or a host-owned external timer path.

This is a bounded local-APIC timer proof, not a general timer subsystem. vCPU1 remains the only AP, vector `0x53` is fixed, the timer is one-shot, and a targeted-MSI watchdog exists only to keep a broken timer path from hanging hosted CI. If the watchdog fires, the milestone fails and cannot be accepted as timer evidence.

Acceptance contract:

- preserve ordinary CI, Rust 1.74 shipped-target MSRV and every existing permanent hosted-KVM workflow;
- create exactly two vCPUs; vCPU1 must begin `KVM_MP_STATE_UNINITIALIZED` and may start only through the existing guest BSP INIT assert/deassert plus SIPI vector `0x08` sequence;
- preserve the first 73 bytes of the integrated AP guest-owned real-mode-to-long-mode transition byte-for-byte;
- after SIPI, AP startup state must remain MP runnable, RIP `0`, CS selector `0x0800`, CS base `0x8000`, CR0.PE clear before the AP performs its own PAE/CR3/EFER/CR0 transition;
- preserve AP long-mode architectural state: stack `0x1ef000`, CS selector `0x08` with L=1, SS selector `0x10`, GDT `0x7000/0x17`, CR3 `0x1000`, and required CR0/CR4/EFER bits;
- install AP timer IDT vector `0x53` at handler GPA `0x13000`; IDTR must be `0x6000/0x53f`;
- AP must software-enable its local APIC and program a one-shot timer with divide configuration `0x0b`, unmasked LVT timer vector `0x53`, and initial count `0x00100000`;
- AP emits readiness `R` and armed `A` while architectural RFLAGS bit1 is set and IF is clear, then executes adjacent `sti; hlt`;
- the local timer must enter vector `0x53`; the handler emits `T`, writes LAPIC EOI and `iretq`; resumed AP mainline stores shared marker `K` at GPA `0x9000`, emits `W`, then emits completion barrier `D`;
- exact AP proof is `ALRATWD`; exact BSP proof is `0IDSMD`; every byte-wide debug-port exit must have exact direction, size, port, count and payload;
- AP completion after `D` requires architectural RFLAGS bit1 and IF set; BSP completion requires bit1 set and IF clear;
- the BSP may observe marker `K` and emit `M`,`D` only after the AP worker has completed the timer path;
- a five-second targeted-MSI watchdog may only prevent a failed timer from wedging CI. Any watchdog intervention is a hard failure with no retry-to-success path;
- worker startup/channel/join failures must preserve their real error or become deterministic verification errors; they must not be coerced into an unrelated outer result type or swallowed;
- KVM-aware integration must independently validate initial/startup AP MP state, AP long-mode/IDT state, ready/armed/completion RFLAGS, both proof streams, every debug exit, shared marker and watchdog=false;
- permanent workflow `Strict KVM two-vCPU AP local timer` must run independently on hosted KVM and require vector `0x53`, TDCR `0x0b`, TMICT `0x100000`, watchdog=false, initial AP MP state `1`, exact SIPI startup state, IDT `0x6000/0x53f`, marker `75`, BSP proof `[48, 73, 68, 83, 77, 68]`, AP proof `[65, 76, 82, 65, 84, 87, 68]`, ready/armed IF clear and completion IF set;
- formatter, Clippy, MSRV, startup, timer, watchdog, proof or architectural-state failures remain hard failures and must not be hidden by changed expectations or skipped hosted-KVM evidence.

The implementation is currently in progress on `milestone/ap-local-lapic-timer` / PR #106. No capability is considered integrated until the final exact candidate passes ordinary CI plus the new permanent AP-local-timer workflow, remains current with `main`, and completes the repository's normal review/merge audit.

## Scope boundary

This milestone deliberately does **not** add:

- periodic LAPIC timer mode, TSC-deadline mode, timer calibration, guest wall clock, PIT, HPET or a general scheduler;
- a BSP local timer, multiple timer vectors, alternate divide/count values, repeated timer interrupts or timer performance/latency claims;
- a third vCPU, CPU hotplug, repeated INIT/SIPI cycles, alternate SIPI/IPI vectors or APIC IDs;
- cross-vCPU TLB shootdown, shared page-table mutation, per-CPU TSS, ring transitions, SYSCALL/SYSRET or a kernel/user privilege model;
- new PCI/virtio/storage behavior, DMA/IOMMU, migration, persistence or durability claims.

## Promotion rule

After the SIPI-started AP local-timer proof is integrated and exact merged-`main` ordinary CI plus all permanent workflows are green, seal the fixed one-shot vector/count proof rather than multiplying timer constants or periodic variants.

The next architecture audit should select a capability that uses the now-integrated SMP startup, IPI, mailbox and per-vCPU timer foundations in a materially new way. Strong candidates are a bounded cross-vCPU TLB-shootdown protocol backed by an actual shared page-table mutation, or a privileged execution boundary with per-CPU TSS/ring transition and executable end-to-end evidence. More timer variants are not a promotion.
