# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `d5eb5cd0783153e0d90fc7d8b2433075a5ceca8c` through PR #135 (`Canonicalize unusable segment restore state`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest-owned scheduling/wait ownership; one-page and bounded multi-page VCPU checkpoints; scheduler/wait checkpoint composition; one coordinated two-VCPU stop-the-world checkpoint; and bounded in-kernel x86 controller checkpoints.

PR #134 seals the bounded full-controller checkpoint at merged commit `13a1fe1635b591afd7b1c2b38bf621a45067008b`. It composes the existing bounded page/VCPU checkpoint with master PIC, slave PIC, IOAPIC and boot-VCPU LAPIC ownership, restores them in dependency order, requires a fresh exact comparison, and resumes both restored slave-PIC and IOAPIC interrupt routes on real KVM. PR #135 changes no capability boundary; it is a correctness repair that canonicalizes only x86 unusable-segment type bit 0 so the same strict restore proof remains deterministic across hosted KVM implementations without weakening usable-segment/register/MSR equality.

The fixed full-controller checkpoint phase is sealed. Do not farm alternate IOAPIC pins, PIC masks, vectors or LAPIC bits merely to extend the phase number.

## Selected milestone — bounded quiescent virtio-blk device checkpoint

The next ownership boundary is one real device model. Compose one exact quiescent virtio-blk device with the existing `BoundedVcpuPageSetCheckpoint`; do not fork page/VCPU restore machinery and do not pretend host file descriptors, registrations or in-flight queue work are migration-safe.

Acceptance contract:

- preserve exact base `d5eb5cd0783153e0d90fc7d8b2433075a5ceca8c`, ordinary CI, Rust 1.74 shipped-target MSRV and every existing permanent hosted-KVM workflow;
- capture only at a non-serviceable `KVM_EXIT_HLT` boundary after a completed virtio-blk request, with no pending MMIO device event/queue notification;
- own exactly one guest page at GPA `0x18000`, which contains the deterministic descriptor/avail/used/header/data/status request state, plus canonical boot-VCPU state and one exact virtio-blk device snapshot at its fixed BAR;
- device snapshot ownership includes negotiated/configuration state, queue indices, ISR state and deterministic in-memory backing; it excludes host fds and external registration state;
- reject non-quiescent device capture or restore, missing/wrong BAR identity, and guest-layer restore mismatch before device mutation;
- deterministic first phase must configure the device, complete one T_OUT request, commit the deterministic sector backing, emit exact debug proof `BWO`, and halt with captured queue indices `avail=1 used=1`;
- checkpoint the one page, VCPU and quiescent device at that first HLT and require the captured device to own the committed backing and queue indices 1/1;
- resume normally and complete one T_IN request from the same queue; this legitimate post-capture execution must advance device queue indices to 2/2 and make fresh page, VCPU and device verification all non-exact;
- restore page/VCPU first and require that layer exact before restoring the device; then restore the same-BAR quiescent device and require a fresh aggregate exact comparison for page, VCPU and device;
- after restore, re-enter real KVM from the restored first HLT and replay the same T_IN request; exact replay proof is `NRD`, queue indices must advance from restored 1/1 to 2/2, and guest readback plus device backing must equal the checkpointed deterministic sector;
- the mutation phase must also produce exact `NRD`, so the proof demonstrates capture → legitimate mismatch → exact restore → executable replay rather than clone equality alone;
- KVM-aware integration must independently validate all three HLT boundaries/RFLAGS bit 1, exact capture/mutation/replay proof bytes, 1/1→2/2 queue transitions, full mismatch→exact comparison, and backing/readback continuity;
- a dedicated permanent hosted-KVM workflow must build `virtio-blk-checkpoint` outside its execution timeout, require `/dev/kvm`, execute the proof under a strict timeout, and require the same page/queue/proof/mismatch/restore/backing evidence; timeout, KVM absence or skipped verification are failures;
- `ROADMAP.md` is part of that workflow path filter, so the final documentation-synchronized candidate must rerun the actual device checkpoint proof.

Implementation is in progress on `milestone/virtio-blk-checkpoint` through PR #136. The checkpoint object and quiescent MMIO-bus capture/verify/restore boundary exist, and the executable proof reuses the integrated atomic virtio-blk queue processor rather than introducing a parallel request path. Real hosted-KVM evidence on the final exact candidate is required before integration.

## Scope boundary

This milestone deliberately does **not** add:

- cross-host serialization, schema/version compatibility, migration save files, pre-copy/post-copy or downtime claims;
- PCI configuration-space checkpointing or reconstruction of PCI discovery state;
- irqfd, ioeventfd, eventfd or other host-fd registration capture/replay;
- in-flight virtqueue/request capture, partially serviced MMIO exits, DMA/IOMMU state or external storage durability semantics;
- multiple virtio devices, multi-device atomic checkpoint transactions or controller+virtio all-in-one restore;
- SMP device checkpointing, cross-VCPU in-flight ownership or whole-VM migration orchestration;
- performance, latency, portability or crash-consistency claims beyond the exact same-process hosted-KVM evidence.

## Promotion rule

After the bounded quiescent virtio-blk checkpoint is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this one-device/one-page queue-state proof rather than farming sectors, queue-index variants or alternate payloads.

The next architecture audit should promote to a materially broader ownership boundary. Strong candidates are explicit event-registration state/reconstruction only if host-resource ownership can be modeled without serializing raw fds, a bounded controller+device checkpoint transaction that proves restore ordering across both subsystems, or later migration serialization/versioning once the owned state surface has a stable schema. Multi-device and SMP device transactions remain separate higher-order frontiers.
