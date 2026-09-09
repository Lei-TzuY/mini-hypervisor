# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `282d9e3b5068ee01d3339c762324cfd7dfde250a` through PR #133 (`Keep SIPI work-dispatch timeout independent of cold build`). Exact ordinary CI and all permanent hosted-KVM workflows for that integrated frontier are settled successfully.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest-owned task scheduling/wait ownership; one-page and bounded multi-page VCPU checkpoints; scheduler/wait checkpoint composition; one coordinated two-VCPU stop-the-world checkpoint; and one bounded page/VCPU/master-PIC/LAPIC controller checkpoint.

PR #132 seals the first kernel-owned controller-state checkpoint at merged commit `0acd69fb33c4e8882f7d19c8317be2228e61754c`. It reuses `BoundedVcpuPageSetCheckpoint`, captures only from the validated non-serviceable `KVM_EXIT_DEBUG` quiescent boundary at RIP `0x10031` with IF clear, owns page `0x30000`, canonical boot-VCPU state, master PIC and LAPIC, proves independent corruption, restores in dependency order and resumes exact `AIMD` interrupt delivery. PR #133 changes no guest-visible behavior; it moves the remaining SIPI work-dispatch cold build outside the unchanged strict 30-second KVM execution budget. The exact `main` workflow matrix after #133 has no failed, queued or in-progress run.

The fixed master-PIC/LAPIC checkpoint phase is sealed. Do not farm alternate PIC masks, marker pages, vectors or LAPIC bits merely to extend the phase number.

## Selected milestone — bounded full in-kernel controller checkpoint

The next state-ownership boundary completes the bounded x86 in-kernel irqchip snapshot surface already selected by the previous architecture audit: compose slave PIC and IOAPIC state with the existing page/VCPU/master-PIC/LAPIC checkpoint. This must extend the established checkpoint object and quiescence boundary rather than fork a second snapshot/restore pipeline.

Acceptance contract:

- preserve exact base `282d9e3b5068ee01d3339c762324cfd7dfde250a`, Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow;
- reuse `BoundedControllerCheckpoint` / `BoundedVcpuPageSetCheckpoint` for page, canonical boot-VCPU, master-PIC and LAPIC ownership; do not duplicate those capture/restore paths;
- model Linux x86 `KVM_GET_IRQCHIP` / `KVM_SET_IRQCHIP` for chip id 1 (slave PIC) and chip id 2 (IOAPIC) using the existing fixed 520-byte outer request; model the IOAPIC payload as base address, ioregsel, id, IRR, pad and 24 redirection entries;
- capture is allowed only at the existing capture-arm PIO plus single-step `KVM_EXIT_DEBUG` boundary at RIP `0x10031` with architectural bit 1 set and IF clear; guest single-step must already be disabled before checkpoint state is owned;
- capture page `0x30000` marker `A`, canonical VCPU state, master PIC IMR `0xfb`, slave PIC IMR `0xfe`, a quiescent IOAPIC with IRR zero, IOAPIC pin8 masked, IOAPIC pin16 configured as fixed vector `0x50`, software-enabled LAPIC SPIV and unmasked ExtINT LINT0;
- reject capture when IOAPIC IRR is nonzero; pending in-kernel interrupt state is outside this bounded quiescent checkpoint contract;
- deliberately corrupt page, VCPU, master PIC, slave PIC, IOAPIC pin16 and LAPIC LINT0 independently; fresh verification must report all six components non-exact;
- restore page/VCPU first and require that layer exact before controller mutation; restore controller state in explicit master PIC → slave PIC → IOAPIC → LAPIC order; any failure stops later restore steps;
- require a fresh six-component exact verification before resumed guest execution;
- after restore, prove both newly-owned controller paths on real KVM: restored marker `A`, GSI8 through slave-PIC vector `0x48` handler `S` with slave+master EOI, bridge byte `B`, then GSI16 through IOAPIC vector `0x50` handler `J`, resumed-main `M`, completion `D`;
- exact executable proof is `ASBJMD` across six byte-wide debug-port exits; the slave-PIC armed, IOAPIC armed and completion states must retain architectural bit 1 and IF;
- KVM-aware integration must independently validate capture RIP/RFLAGS, six mismatch→exact transitions, PIC IMRs, IOAPIC pin16, LAPIC semantics, all six proof exits and both resumed interrupt routes;
- a dedicated permanent hosted-KVM workflow must build `full-controller-checkpoint` outside the unchanged 30-second execution timeout, then require the same capture, restore, `ASBJMD`, controller and RFLAGS evidence; `/dev/kvm` absence, timeout, changed expected values or skipped verification are not successful outcomes;
- `ROADMAP.md` is part of that workflow path filter so the final documentation-synchronized candidate must independently rerun the full-controller proof.

Implementation is in progress on `milestone/full-controller-checkpoint`. The branch extends the existing irqchip snapshot UAPI with slave-PIC and IOAPIC state, composes them into the existing bounded checkpoint, and includes an executable two-route restore proof. Real-KVM evidence is required before this milestone may be integrated.

## Scope boundary

This milestone deliberately does **not** add:

- arbitrary irqchip serialization beyond master PIC, slave PIC, IOAPIC and the existing boot-VCPU LAPIC;
- irqfd/eventfd/ioeventfd registration checkpointing, replay or host-fd migration;
- PCI/virtio device-state checkpointing, DMA/IOMMU state or device inflight replay;
- capture or replay from arbitrary in-flight PIO/MMIO service exits;
- SMP controller checkpointing, cross-VCPU pending interrupt replay or a whole-VM migration transaction;
- migration serialization/versioning, cross-host compatibility, pre-copy/post-copy or crash-consistency claims;
- performance, downtime or portability claims beyond the exact hosted-KVM evidence.

## Promotion rule

After the bounded full-controller checkpoint is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this fixed controller-coverage proof rather than farming extra IOAPIC pins, PIC masks or vectors.

The next architecture audit should promote to a materially broader ownership boundary. Strong candidates are bounded PCI/virtio device-state checkpoint composition or explicit event-registration state/replay only if ownership, restore ordering and resumed real-KVM behavior can be represented without pretending host file descriptors or in-flight device work are migration-safe. SMP controller checkpointing and migration serialization/versioning remain separate higher-order frontiers.
