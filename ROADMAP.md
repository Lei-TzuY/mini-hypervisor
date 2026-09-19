# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `3abf5a42bfa2cd610718f4ef51842f0d5eb8284b` through PR #143 (`Build syscall partial proof outside execution timeout`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; fd-free ioeventfd/irqfd reconstruction around restore; and canonical versioned byte schemas for page+VCPU, full-controller, full-controller+one-quiescent-virtio-blk, and the fd-free host-registration reconstruction specification.

PR #138 sealed the canonical little-endian v1 page+VCPU schema. PR #139 sealed reconstruction of fresh process-local host registrations after restore. PR #140 sealed the canonical v1 full-controller envelope. PR #141 extended that byte ownership boundary across one quiescent virtio-blk device and deterministic backing. PR #142 then serialized the fd-free semantic host-registration reconstruction specification itself and proved fresh ioeventfd/irqfd lifecycles for both mutation and restored replay. PR #143 did not change checkpoint semantics; it moved cold binary compilation outside the syscall partial-progress execution timeout so build time cannot masquerade as guest execution failure.

On exact `main=3abf5a42bfa2cd610718f4ef51842f0d5eb8284b`, ordinary CI and every workflow applicable to that head completed successfully. The versioned host-registration reconstruction phase is sealed. Do not farm alternate doorbells, GSIs, reserved-byte corruption cases, or equivalent single-registration variants merely to extend the phase number.

## Selected milestone — one canonical versioned checkpoint transaction envelope

The next ownership boundary is the fact that the integrated full-controller+virtio-blk checkpoint bytes and fd-free host-registration bytes are still carried as two independent caller-owned blobs. This milestone binds those two already-sealed schemas into one canonical outer transaction envelope without duplicating either nested schema's semantic fields.

Implementation continues on `milestone/versioned-checkpoint-transaction-envelope`, created from exact green `main=3abf5a42bfa2cd610718f4ef51842f0d5eb8284b`.

Acceptance contract:

- preserve the exact base behavior, Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow;
- define a canonical little-endian v1 outer transaction header with explicit magic, version, x86-64 architecture identifier, header length, total length, nested checkpoint length, nested registration length, zero flags and zero reserved fields;
- the outer envelope owns framing only: it must embed the existing canonical `VersionedFullControllerVirtioBlkCheckpointV1` bytes and existing canonical `VersionedHostRegistrationSpecV1` bytes rather than duplicate page, VCPU, controller, device, BAR, backing, doorbell or GSI fields;
- never serialize raw ioeventfd/irqfd/eventfd descriptors, Linux/KVM padding, live registrations, registration generations, thread handles or host object identities;
- outer decode must fail closed on bad magic/version/architecture/header/total length, zero or impossible nested lengths, length arithmetic overflow, non-zero flags/reserved fields and truncation;
- nested checkpoint and host-registration decode/materialization failures remain visible as nested errors rather than being accepted or normalized by the outer envelope;
- canonical outer encode→decode→re-encode identity is required;
- executable proof must capture the integrated full-controller+virtio-blk checkpoint and semantic host-registration specification, encode exactly one outer byte stream, discard the original process-local checkpoint/spec objects, then decode and materialize both nested objects only from that outer stream;
- fresh ioeventfd/irqfd registrations used for mutation and restored replay must be reconstructed only from the host-registration spec decoded from the transaction;
- preserve the sealed restore dependency order: guest pages/VCPU/controllers first, then virtio-blk device state, then fresh host acceleration before a request is executed;
- mutation and replay must each traverse reconstructed ioeventfd/irqfd acceleration, prove exactly one doorbell event and one irqfd signal, preserve exact `NIARD` proof, preserve the intentional full mismatch before restore, prove full exactness after restore, and advance restored queue state from 0/0 to 1/1;
- deterministic virtio-blk backing and guest readback must remain identical after replay;
- schema evidence must expose outer version/encoded length/canonical status plus both nested versions and lengths, page/MSR counts, BAR/backing metadata and the decoded doorbell/GSI tuple;
- deterministic unit coverage must include outer framing corruption and truncation while existing nested schema suites retain their own semantic corruption coverage;
- KVM-aware integration must independently validate the outer byte relation `total = 48 + checkpoint + registration`, nested metadata, decoded registration semantics, exact controller/device restore, both fresh accelerated phases and backing continuity;
- add a dedicated permanent hosted-KVM workflow whose path filter includes the outer schema, both nested schema/runtime dependencies, proof binary, integration test, ROADMAP and the workflow itself;
- build the proof binary outside the execution timeout; the 30-second budget applies only to execution of the already-built KVM proof, so cold compilation cannot masquerade as a guest execution failure;
- schema, compatibility, reconstruction, dependency ordering, ioeventfd/irqfd delivery, controller/device state, replay, queue progression or backing failures remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** serialize raw descriptors or live kernel registration state; add another equivalent device/doorbell/GSI; add irqfd resample semantics or arbitrary KVM GSI routing; checkpoint an in-flight virtio request; add a second device or SMP-wide migration transaction; claim cross-host/live migration compatibility; provide external-storage durability/crash-consistency semantics; or make performance/downtime claims.

The outer bytes are a canonical ownership transaction for the already-bounded current-host-compatible nested schemas. They are not a portable representation of Linux kernel object state and do not constitute a live-migration protocol.

## Promotion rule

After the versioned checkpoint transaction is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal the one-device/one-registration atomic envelope rather than adding alternate framing constants or more corruption offsets.

The next architecture audit should prefer a genuinely larger ownership boundary. Strong candidates are a bounded multi-device or coordinated multi-vCPU checkpoint transaction only when quiescence, dependency ordering, host-registration reconstruction and external-state semantics can be proven coherently. Cross-host/live migration, performance/downtime guarantees and external-storage crash consistency remain separate later frontiers.
