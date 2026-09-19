# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `26a480994cef2524158fa2877d72be8823f987b4` through PR #141 (`Encode full-controller virtio-blk checkpoints with a versioned schema`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; fd-free ioeventfd/irqfd reconstruction around restore; and canonical versioned byte schemas for page+VCPU, full-controller, and full-controller+one-quiescent-virtio-blk checkpoints.

PR #138 sealed the canonical little-endian v1 page+VCPU schema. PR #139 sealed process-local reconstruction of host registrations from fd-free semantic specifications after exact checkpoint restore. PR #140 sealed the canonical v1 full-controller envelope without serializing Linux padding. PR #141 then extended the versioned ownership boundary across one quiescent virtio-blk device and its deterministic in-memory backing while preserving controller-before-device restore ordering.

Exact merged-main ordinary CI and every applicable permanent hosted-KVM workflow are green on `26a480994cef2524158fa2877d72be8823f987b4`: 42 of 42 workflows completed successfully, including the dedicated versioned full-controller+virtio-blk proof and host-registration reconstruction proof.

The one-device versioned checkpoint phase is sealed. Do not farm alternate BAR values, backing patterns, corruption offsets, or another equivalent one-device envelope merely to extend the phase number.

## Selected milestone — versioned fd-free host-registration reconstruction

The next ownership boundary is the acceleration metadata that intentionally remains outside serialized VM/device state. The runtime can already reconstruct fresh ioeventfd/irqfd registrations from an in-process fd-free `HostRegistrationSpec` after checkpoint restore, but that registration specification itself has no explicit byte compatibility boundary. This milestone serializes only the semantic reconstruction contract, never raw descriptors or live registrations, and composes it with the integrated versioned full-controller+virtio-blk checkpoint transaction.

Acceptance contract:

- preserve exact base `26a480994cef2524158fa2877d72be8823f987b4`, ordinary CI, Rust 1.74 shipped-target MSRV and every applicable permanent hosted-KVM workflow;
- define a canonical fixed-size little-endian v1 host-registration spec with explicit magic, version, header length, total length, zero flags and zero reserved fields;
- serialize only semantic reconstruction fields: doorbell guest-physical address, doorbell width, doorbell datamatch and interrupt GSI;
- never serialize raw ioeventfd/irqfd/eventfd descriptors, Linux/KVM padding, registration handles, host-registration generations, thread handles or host object identities;
- decode must reject wrong magic/version/header/total length, non-zero flags/reserved fields, malformed/truncated bytes and any semantic combination rejected by the existing validated `HostRegistrationSpec`;
- canonical encode→decode→re-encode identity is required;
- the executable proof must capture the integrated full-controller+virtio-blk checkpoint, encode the VM/device checkpoint and host-registration spec, drop the original process-local reconstruction state, decode/materialize the serialized state, and only then construct fresh host registrations;
- reconstruction must allocate fresh ioeventfd/irqfd resources from the decoded semantic spec; serialized bytes must remain fd-free and generation-free;
- preserve the sealed guest/page/VCPU/controller-first then virtio-blk restore ordering before reconstructed host acceleration is used;
- run both the deliberate mutation request and restored replay request through reconstructed ioeventfd/irqfd registrations, not through a userspace-MMIO fallback;
- mutation and replay must each prove exactly one doorbell event and exactly one irqfd signal while preserving the existing request proof `NIARD`, queue progression, interrupt lifecycle and deterministic backing/readback continuity;
- mutation and replay must each enter a fresh reconstruction lifecycle from the decoded semantic spec, with the mutation registrations deassigned and dropped before replay reconstruction begins;
- schema evidence must expose version, encoded length, canonical-roundtrip status and the decoded doorbell/GSI tuple;
- deterministic unit coverage must include envelope corruption, flags/reserved corruption, truncation and semantic host-registration invalidity;
- KVM-aware integration must independently validate the schema metadata, two separate accelerated request phases reconstructed from the decoded spec, exact full-controller+virtio-blk restore and backing continuity;
- add a dedicated permanent hosted-KVM workflow whose path filter includes `ROADMAP.md`, the schema/runtime files, executable binary, integration test and the workflow itself;
- build the proof binary outside the execution timeout so cold compilation cannot masquerade as a KVM execution failure;
- schema, compatibility, restore ordering, registration reconstruction, generation freshness, ioeventfd/irqfd delivery, controller/device state, replay or backing failures remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

Implementation continues on `milestone/versioned-host-registration-reconstruction`.

## Scope boundary

This milestone deliberately does **not** serialize raw file descriptors, live eventfd counters, live ioeventfd/irqfd kernel registrations or host-registration generations; add irqfd resample semantics; add arbitrary KVM GSI routing; checkpoint an in-flight virtio request; claim cross-host migration compatibility; add a second device or SMP-wide migration transaction; provide external-storage durability/crash-consistency semantics; or make performance/downtime claims.

The serialized registration bytes describe how current-host acceleration is to be reconstructed. They are not a portable representation of Linux kernel object state.

## Promotion rule

After versioned host-registration reconstruction is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal the single-registration reconstruction phase rather than farming alternate doorbells, GSIs or corruption offsets.

The next architecture audit should prefer a genuinely larger ownership boundary. Strong candidates are one canonical outer transaction that binds the versioned full-controller+virtio-blk checkpoint and versioned host-registration spec into one atomic envelope, or a bounded multi-device/multi-vCPU ownership set only when quiescence, dependency ordering and external-state semantics can be proven coherently. Cross-host/live migration, performance/downtime and external-storage crash consistency remain separate later frontiers.
