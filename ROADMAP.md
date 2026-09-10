# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `e3096f04beabcdedace85dcace18c8bb9b7b2920` through PR #136 (`Checkpoint quiescent virtio-blk device state`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest-owned scheduling/wait ownership; one-page and bounded multi-page VCPU checkpoints; scheduler/wait checkpoint composition; one coordinated two-VCPU stop-the-world checkpoint; bounded in-kernel x86 controller checkpoints; and one bounded quiescent virtio-blk device checkpoint.

PR #134 seals the bounded full-controller checkpoint at merged commit `13a1fe1635b591afd7b1c2b38bf621a45067008b`. PR #135 is a correctness repair that canonicalizes only x86 unusable-segment type bit 0 so strict restore proof remains deterministic across hosted KVM implementations without weakening usable-segment/register/MSR equality.

PR #136 seals the one-device checkpoint at merged commit `e3096f04beabcdedace85dcace18c8bb9b7b2920`. It composes the existing bounded page/VCPU checkpoint with exactly one quiescent virtio-blk snapshot at the fixed BAR, proves legitimate post-capture mismatch, restores page/VCPU state before device state, requires a fresh exact aggregate comparison, and re-enters real KVM to replay the request with deterministic backing/readback continuity. Exact merged-main ordinary CI and the permanent `Strict KVM virtio-blk device checkpoint restore` workflow are green.

The one-device/one-page virtio-blk checkpoint phase is sealed. Do not farm alternate sectors, queue indices, BAR aliases or payload variants merely to extend the phase number.

## Selected milestone — atomic full-controller + virtio-blk checkpoint transaction

The next ownership boundary is a single transaction spanning guest page/VCPU state, the full in-kernel interrupt-controller aggregate and one quiescent virtio-blk device. Reuse the sealed controller and device checkpoint primitives; do not fork page/VCPU restore machinery and do not restore page/VCPU state twice through nested checkpoint objects.

Acceptance contract:

- preserve exact base `e3096f04beabcdedace85dcace18c8bb9b7b2920`, ordinary CI, Rust 1.74 shipped-target MSRV and every existing permanent hosted-KVM workflow;
- own exactly one `BoundedFullControllerCheckpoint`, one exact BAR identity and one quiescent virtio-blk device snapshot; no host fd or external registration state is captured;
- capture only a quiescent virtio-blk queue state with no pending MMIO device event and one exact guest page at GPA `0x18000`;
- restore the full page/VCPU/controller aggregate first and require its fresh comparison to be exact before any device-state mutation; a non-exact controller restore must hard-fail and the device restore callback must not run;
- only after the controller aggregate is exact may the virtio-blk state be restored; then perform a fresh aggregate comparison requiring page, VCPU, master PIC, slave PIC, IOAPIC, LAPIC and device state all exact;
- deterministic capture starts from negotiated/enabled virtio-blk queue indices `avail=0 used=0` at a non-serviceable single-step debug boundary with RFLAGS bit 1 and IF set;
- mutation execution must submit queue 0, service one deterministic request, traverse the existing GSI0/PIC/LAPIC ExtINT completion interrupt path, acknowledge the ISR, produce exact proof `NIARD`, assert/deassert the line exactly once and reach the exact quiescent request boundary;
- after legitimate request execution plus explicit controller corruption, fresh verification must report the owned page, VCPU, master PIC, slave PIC, IOAPIC, LAPIC and device all non-exact;
- exact restore must return the device to quiescent queue indices `0/0`, leave no pending device event and restore all controller layers before device mutation;
- re-enter real KVM after restore and replay the same request; exact replay proof is `NIARD`, line lifecycle is one assert and one deassert, queue indices advance from restored `0/0` to `1/1`, and completion traverses the restored controller path before returning to the guest;
- exact request MMIO metadata is one queue-notify write at BAR+`0x100` followed by two one-byte ISR reads; notify payload is queue 0 and completion metadata remains descriptor 0, sector 0 and length `VIRTIO_BLK_SECTOR_SIZE + 1`;
- replay backing and guest readback must both equal the deterministic sector, and final replay RFLAGS must retain architectural bit 1 and IF;
- KVM-aware integration must independently validate capture state, complete mismatch, complete exact restore, mutation/replay proof bytes, line lifecycle, queue transitions, RFLAGS and backing/readback continuity;
- a dedicated permanent hosted-KVM workflow must build the combined proof binary outside its execution timeout, require `/dev/kvm`, execute under a strict timeout and require the same combined controller/device/mutation/restore/replay evidence; KVM absence, timeout or skipped verification are failures;
- `ROADMAP.md` is included in that workflow path filter so the final documentation-synchronized candidate reruns the real combined checkpoint proof;
- restore-order, quiescence, BAR identity, MMIO metadata, interrupt lifecycle, comparison or replay failures remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

Implementation is in progress on `milestone/full-controller-virtio-blk-checkpoint` through PR #137. The combined checkpoint primitive, fail-closed controller-before-device ordering regression and real-KVM fixture exist. Final integration requires the dedicated hosted-KVM proof, ordinary CI and every applicable permanent workflow to pass on the exact documentation-synchronized candidate before merge.

## Scope boundary

This milestone deliberately does **not** add:

- cross-host serialization, migration schemas/version negotiation, save files, pre-copy/post-copy or downtime claims;
- host-fd snapshotting, irqfd/ioeventfd/eventfd registration capture or recreation;
- in-flight virtqueue/request checkpointing, partially serviced MMIO exits, DMA/IOMMU state or external storage durability semantics;
- PCI configuration-space checkpointing or PCI topology reconstruction;
- multiple virtio devices, multi-device atomic transactions, multiple VCPUs or SMP device checkpointing;
- performance, latency, portability or crash-consistency claims beyond exact same-process hosted-KVM evidence.

## Promotion rule

After the full-controller + one-device transaction is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this fixed combined checkpoint rather than farming alternate controller bits, sectors, BARs or request payloads.

The next architecture audit should promote to a materially broader ownership boundary. Strong candidates are an explicit serializable/versioned checkpoint schema with compatibility validation, host-resource registration reconstruction that models recreatable resources without serializing raw fds, or a bounded multi-device/SMP checkpoint transaction only when ownership and quiescence can be proven coherently. Performance/downtime, live migration and external-storage crash consistency remain separate frontiers requiring their own evidence.
