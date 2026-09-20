# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `acd19cfb4bd0fe3abb436df208a5f6653224426a` through PR #144 (`Bind checkpoint state into one versioned transaction`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; fd-free ioeventfd/irqfd reconstruction around restore; canonical versioned byte schemas for page+VCPU, full-controller, full-controller+one-quiescent-virtio-blk, and host-registration reconstruction; and one canonical outer transaction that owns the sealed checkpoint and registration blobs together.

PR #144 sealed the one-device/one-registration transaction envelope. The exact merged-main commit `acd19cfb4bd0fe3abb436df208a5f6653224426a` completed ordinary CI plus every applicable permanent hosted-KVM workflow successfully, including the strict versioned checkpoint transaction proof. Do not farm framing constants, equivalent single-registration variants, or more corruption offsets merely to extend that phase.

## Selected milestone — two virtio-blk devices under one full-controller checkpoint

The next ownership boundary is multi-device checkpoint coordination. Before widening the canonical transaction schema or the host-registration schema from one tuple to a collection, the runtime must first prove that one full-controller checkpoint can own and restore two independent quiescent virtio-blk devices without partial device mutation or ambiguous restore ordering.

Implementation continues on `milestone/two-virtio-blk-controller-checkpoint`, created from exact green `main=acd19cfb4bd0fe3abb436df208a5f6653224426a`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow already green on the base;
- own exactly two distinct 4KiB-aligned virtio-blk BAR identities in canonical ascending BAR order;
- capture one existing `BoundedFullControllerCheckpoint` plus two independently captured quiescent `VirtioBlkDevice` snapshots; no second controller or second VCPU is introduced in this slice;
- reject duplicate or misaligned BAR ownership before capture;
- both devices must exist and be quiescent at capture;
- comparison must retain per-BAR device exactness rather than collapse two devices into one opaque boolean;
- restore must preflight both live devices before mutating page/VCPU/controller state or either device, so a missing or in-flight second device cannot leave the first device partially restored;
- after preflight, restore page/VCPU/full-controller state through the existing dependency order and require exact controller restoration before either device snapshot is applied;
- restore both device snapshots atomically with respect to validation: validate both snapshot identities/quiescence and both live device identities/quiescence before mutating either live device;
- restore the two devices in deterministic ownership order and verify both independently after restoration;
- controller restore mismatch remains a hard gate that prevents device restore;
- deterministic real-KVM proof must capture two distinct valid quiescent device states, corrupt checkpoint-owned page/VCPU/controller state and both devices, prove controller plus both devices mismatch, restore all owned state exactly, and resume the restored guest through the existing full-controller `ASBJMD` proof;
- the two deterministic device states must remain distinguishable after restore: BAR `0x10000000` has status `0x01`, BAR `0x10001000` has status `0x03`;
- unit coverage must prove canonical BAR ordering, duplicate/misaligned BAR rejection, controller-exact gating, and that failure to preflight the second live device leaves the first device unchanged;
- KVM-aware integration must independently validate controller/page/VCPU mismatches, both device mismatches, exact restoration of every controller component and both devices, restored status identity, final `ASBJMD` proof and interrupt-enabled completion state;
- add a dedicated permanent hosted-KVM workflow whose path filter covers the checkpoint ownership type, two-device atomic bus restore, proof binary, integration test, ROADMAP and the workflow itself;
- build the proof binary outside the execution timeout; the timeout applies only to execution on KVM;
- missing/non-quiescent devices, identity mismatch, controller mismatch, restore-order failure, device mismatch, KVM proof failure or architectural-state failure remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** define a new multi-device versioned byte schema, expand the fd-free host-registration schema to multiple tuples, change the outer transaction framing, add a second VCPU, checkpoint an in-flight virtio request, serialize raw eventfds/irqfds/ioeventfds, claim cross-host/live migration compatibility, add external-storage durability semantics, or make performance/downtime claims.

The two-device checkpoint is the executable runtime prerequisite for a later multi-device transaction. It proves ownership and restore ordering before serialization is widened.

## Promotion rule

After the two-device full-controller checkpoint is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal the fixed two-device runtime ownership proof rather than adding a third identical device.

The next architecture audit should prefer a canonical bounded multi-device transaction: extend the sealed full-controller+virtio-blk representation to a canonical two-device ownership set, extend fd-free host-registration reconstruction to the corresponding bounded registration set, and bind both into one outer transaction only if quiescence, canonical ordering, fresh registration reconstruction, exact restore and replay can all be proven coherently. Coordinated multi-VCPU transaction ownership remains a separate frontier because secondary-VCPU serialization/controller ownership is not yet a sealed nested schema.
