# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `de239a1d36bb5adecf090acf5e3964bc37a78ccb` through PR #137 (`Checkpoint full controller and virtio-blk state atomically`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; one-page and multi-page VCPU checkpoints; scheduler/wait checkpoint composition; coordinated two-VCPU checkpointing; bounded full in-kernel controller checkpoints; one quiescent virtio-blk device checkpoint; and one atomic full-controller + virtio-blk checkpoint transaction.

PR #137 seals that combined transaction at merged commit `de239a1d36bb5adecf090acf5e3964bc37a78ccb`. It owns one full page/VCPU/controller aggregate plus one quiescent virtio-blk snapshot, restores controller state before device mutation, requires fresh exact aggregate comparison, re-enters real KVM, and proves deterministic request replay, interrupt completion and backing/readback continuity. Exact merged-main ordinary CI and every applicable permanent hosted-KVM workflow are green.

That fixed same-process controller+device checkpoint phase is sealed. Do not farm alternate sectors, queue indices, controller bits, BAR aliases or request payloads merely to extend the phase number.

## Selected milestone — versioned page + VCPU checkpoint schema

The next boundary is an explicit byte representation rather than a process-local Rust object. This first serialization slice is deliberately restricted to the already-sealed bounded page+VCPU checkpoint aggregate so format invariants, compatibility validation and fail-closed decoding can be proven before controller/device or host-registration state is added.

Acceptance contract:

- preserve exact base `de239a1d36bb5adecf090acf5e3964bc37a78ccb`, ordinary CI, Rust 1.74 shipped-target MSRV and every applicable permanent hosted-KVM workflow;
- define a canonical little-endian v1 header with fixed magic, version, x86_64 architecture id, header length, total length, page size, bounded counts, zero flags and zero reserved fields;
- encode pages, all 18 general registers, semantic special-register fields and bounded MSR values field-by-field; never serialize native Rust/KVM struct memory;
- require canonical page/MSR ordering, aligned non-overflowing GPAs, exact lengths, zero reserved bytes, valid segment booleans/DPL/type and bounded counts;
- reject unknown version/architecture, corruption, truncation, trailing bytes, duplicate/noncanonical pages or MSRs and invalid semantic fields before any VM mutation;
- materialization must revalidate serialized MSR indices against the current host-supported MSR list before producing an existing `BoundedVcpuPageSetCheckpoint`;
- raw-to-typed snapshot constructors remain crate-internal;
- executable proof must capture the existing bounded page+VCPU checkpoint, encode bytes, discard the original checkpoint object, decode/materialize, corrupt live pages/VCPU, restore through the decoded checkpoint, verify exact state and resume the deterministic guest on real KVM;
- exact deterministic schema proof owns pages `0x30000`, `0x31000` and `0x1fe000`; corruption must report control/data/stack/VCPU all non-exact, restore must report all four exact, proof is `ABCR`, capture RIP is `0x10013`, terminal RIP is `0x1002d`, and architectural RFLAGS bit 1 remains set;
- KVM-aware integration and a dedicated permanent hosted-KVM workflow must independently prove the same encode/decode/materialize/restore/resume contract; the workflow path filter includes `ROADMAP.md` so the documentation-synchronized final candidate reruns the strict proof;
- decoder/compatibility, MSR-host validation, ordering, bounds, corruption detection, exact restore or resumed execution failures remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

Implementation is in progress on `milestone/versioned-page-vcpu-checkpoint-schema` through PR #138. Candidate `cff5fe5760c211d1dacf743fc67d754bbca0935c` has already passed ordinary CI, Rust 1.74 MSRV, all applicable permanent workflows and the dedicated real-KVM schema proof with encoded size 12808 bytes and exact `ABCR` restore/resume evidence. This ROADMAP synchronization changes the exact candidate and therefore requires the full applicable workflow set to rerun before integration.

## Scope boundary

This milestone deliberately does **not** add:

- controller/device serialization, host-fd serialization, irqfd/ioeventfd/eventfd registration serialization or PCI-topology encoding;
- cross-host migration compatibility claims, save-file durability, pre-copy/post-copy, downtime or live-migration semantics;
- compression, encryption, signatures, performance or portability claims;
- unbounded allocations, arbitrary guest page counts or arbitrary MSR sets;
- multi-device/SMP atomic serialized transactions in this slice.

## Promotion rule

After the versioned page+VCPU schema is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal v1 rather than farming additional corruption bytes or page-order variants.

The next architecture audit should broaden one ownership boundary coherently. Strong candidates are extending the schema to the already-sealed controller/device aggregate with explicit version compatibility, or reconstructing fd-free host registrations after restore. Those are separate surfaces and must be rebased/revalidated against the newly integrated schema before further integration. Cross-host/live migration, performance/downtime claims and external-storage crash consistency remain later frontiers.
