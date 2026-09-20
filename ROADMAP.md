# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `8cc11e19b1225b4d74af02866e2bc55f0b21bde7` through PR #145 (`Checkpoint two virtio-blk devices with full controller state`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; fd-free ioeventfd/irqfd reconstruction around restore; canonical versioned byte schemas for page+VCPU, full-controller, full-controller+one-quiescent-virtio-blk, and host-registration reconstruction; one canonical one-device outer checkpoint transaction; and one full-controller checkpoint that owns exactly two independent quiescent virtio-blk devices with atomic validation-before-mutation restore semantics.

PR #145 sealed the fixed two-device runtime ownership prerequisite. Exact merged-main commit `8cc11e19b1225b4d74af02866e2bc55f0b21bde7` completed all 45 push-triggered workflows successfully, including ordinary `CI` and the strict two-virtio-blk full-controller KVM proof. Do not farm a third identical device, status variants, BAR permutations or additional restore-order corner cases merely to extend this phase.

## Selected milestone — versioned two-device full-controller checkpoint

The next compatibility boundary is the already-integrated two-device runtime checkpoint. Before defining a multi-registration outer transaction, the process-local `BoundedFullControllerTwoVirtioBlkCheckpoint` must cross one canonical byte boundary and materialize back into the same two-device ownership contract. This is the serialization prerequisite for a later bounded two-registration transaction; it does not pretend that dual accelerated request replay already exists.

Implementation continues on `milestone/versioned-two-virtio-blk-checkpoint` through PR #146 from exact green `main=8cc11e19b1225b4d74af02866e2bc55f0b21bde7`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow green on the exact base;
- define a canonical little-endian v1 full-controller + exactly-two-virtio-blk envelope with explicit magic, version, x86-64 architecture identifier, fixed header length, total length, nested-controller length, fixed device-state length, exact device count, zero flags and zero reserved state;
- reuse the sealed `VersionedFullControllerCheckpointV1` payload and the existing semantic virtio-blk checkpoint encoding rather than duplicate page/VCPU/PIC/IOAPIC/LAPIC or device-model semantics;
- own exactly two device payloads in strictly increasing canonical BAR order; duplicate, reversed or misaligned BAR ownership must fail closed;
- preserve both model-bound deterministic backing stores and every semantic queue/device field accepted by the existing virtio-blk checkpoint validator;
- reject bad magic/version/architecture/header/total/controller/device lengths, wrong device count, non-zero flags/reserved state, arithmetic overflow, truncation, malformed nested controller state and malformed device state;
- materialization must re-run current-host MSR compatibility through the nested controller schema and re-run both virtio-blk semantic validators before reconstructing the process-local checkpoint;
- executable transport must capture the real two-device checkpoint, encode it, drop both the encoder-side schema and original process-local checkpoint, decode only from bytes, require byte-for-byte canonical decode→re-encode identity, materialize on the current host, and only then continue;
- the materialized checkpoint must preserve BARs `0x10000000` and `0x10001000` and distinct statuses `0x01` and `0x03`;
- the existing deterministic real-KVM proof must still deliberately mismatch page/VCPU/master-PIC/slave-PIC/IOAPIC/LAPIC and both devices, restore every owned component exactly, preserve per-BAR exactness, and resume through proof `ASBJMD` with architectural RFLAGS bit 1 and IF set;
- deterministic unit coverage must prove fixed envelope shape and canonical BAR ownership failures; KVM integration must prove schema metadata plus the complete existing corruption/restore/resume contract;
- add a dedicated permanent hosted-KVM workflow covering schema/runtime/proof/test/roadmap surfaces, with proof binary build outside the 30-second KVM execution timeout;
- schema, compatibility, device semantics, restore ordering, exactness or KVM proof failures remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** expand fd-free host registration from one descriptor to a collection, change the existing one-device outer transaction, add a second accelerated request lifecycle, checkpoint an in-flight request, serialize raw file descriptors or Linux/KVM padding, claim cross-host/live migration compatibility, add external-storage durability semantics, add a second VCPU, or make performance/downtime claims.

The two backing arrays remain the repository's bounded deterministic in-memory virtio-blk model. Their serialization is not a claim that arbitrary host files or block devices can be snapshotted crash-consistently.

## Promotion rule

After the versioned two-device checkpoint is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this fixed-two-device byte schema rather than adding another device count or alternate framing.

The next architecture frontier is **two-device reconstructed acceleration**: prove a bounded canonical pair of fd-free host-registration descriptors, fresh ioeventfd/irqfd reconstruction and independent device/request ownership with deterministic cleanup. Only after that executable registration/replay lifecycle is sealed should the outer transaction be widened to bind the versioned two-device checkpoint and the two-registration set into one canonical transaction. Coordinated multi-vCPU ownership, cross-host/live migration, external-storage crash consistency and performance/downtime remain separate later frontiers.
