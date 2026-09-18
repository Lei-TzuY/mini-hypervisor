# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `1b57e441dd264ea9e51c37ce659140366444268c` through PR #140 (`Encode full-controller checkpoints with a versioned schema`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; reconstruction of fd-free ioeventfd/irqfd host registrations around exact restore; and explicit versioned checkpoint schemas for page+VCPU and the full in-kernel controller aggregate.

PR #138 sealed canonical little-endian v1 page+VCPU checkpoint bytes with fail-closed compatibility/semantic validation and current-host MSR revalidation during materialization. PR #139 sealed the recreatable host-registration boundary: raw fds remain outside serialized ownership, while doorbell/GSI semantics rebuild fresh ioeventfd/irqfd generations around restore. PR #140 then sealed a canonical v1 full-controller envelope over the page+VCPU payload plus semantic master PIC, slave PIC, IOAPIC and LAPIC state without serializing Linux padding.

Exact merged-main ordinary CI and every applicable permanent hosted-KVM workflow are green on `1b57e441dd264ea9e51c37ce659140366444268c`.

The versioned full-controller phase is sealed. Do not farm alternate corruption offsets, equivalent envelope fields or another controller-only encoding merely to extend the phase number.

## Selected milestone — versioned full-controller + virtio-blk atomic checkpoint schema

The next ownership boundary is the already-integrated atomic full-controller + one quiescent virtio-blk checkpoint. The runtime can already capture, deliberately diverge, restore and replay page/VCPU/controller/device state coherently, but that aggregate is still process-local. This milestone extends the explicit byte compatibility boundary across the device model and deterministic backing while preserving the already-sealed controller-before-device restore ordering.

Acceptance contract:

- preserve exact base `1b57e441dd264ea9e51c37ce659140366444268c`, ordinary CI, Rust 1.74 shipped-target MSRV and every applicable permanent hosted-KVM workflow;
- define a canonical little-endian v1 outer envelope with explicit magic, version, architecture, fixed header length, total length, nested-controller length, device length, zero flags and zero reserved fields;
- reuse the sealed v1 full-controller payload as the nested controller state rather than duplicating page, VCPU, PIC, IOAPIC or LAPIC encodings;
- serialize only semantic virtio-blk state: BAR, feature selectors and negotiated features, status, queue enable/select/size, queue addresses, queue indices, ISR status and deterministic backing bytes;
- encode and validate the model-bound sector size, capacity and backing length so bytes for a different device model fail closed;
- never serialize raw Linux/KVM padding, file descriptors, ioeventfd/irqfd registrations or host-registration generations;
- reject wrong magic/version/architecture/header/total/nested/device lengths, non-zero flags/reserved fields, invalid booleans, nested-controller corruption, BAR/model mismatch, unsupported device state, length overflow and malformed/truncated backing;
- materialization must re-run the nested schema's current-host MSR compatibility validation and the virtio-blk semantic validation before reconstructing the process-local checkpoint;
- the executable transport proof must capture the atomic checkpoint, encode it, drop the original process-local schema/checkpoint, decode the bytes, require canonical decode→re-encode identity, materialize on the current host, and only then enter mutation/restore/replay;
- preserve the existing guest/page/VCPU/controller-first then device restore ordering; device mutation must not occur if controller restoration is not exact;
- reuse the sealed full-controller+virtio-blk guest and exact request lifecycle rather than create a parallel guest path;
- the KVM proof must still show the captured quiescent queue at 0/0, deliberate mismatch of page/VCPU/master-PIC/slave-PIC/IOAPIC/LAPIC/device state, exact seven-component restore, mutation and replay proof `NIARD`, one assert/deassert lifecycle per request, restored replay advancement to 1/1, and deterministic backing/readback continuity;
- schema evidence must expose version, encoded length, page/MSR counts, BAR, backing length and canonical-roundtrip status;
- deterministic unit coverage must include envelope/header corruption, nested-controller corruption, device-model/reserved corruption, BAR/state compatibility, host-MSR revalidation and truncation;
- KVM-aware integration must independently validate both schema metadata and the complete existing atomic mutation/restore/replay contract;
- add a dedicated permanent hosted-KVM workflow whose path filter includes `ROADMAP.md`, the schema/runtime/device files, executable binary, integration test and the workflow itself;
- build the proof binary outside the unchanged execution timeout so cold compilation cannot masquerade as a KVM execution failure;
- schema, compatibility, restore-ordering, controller/device state, queue lifecycle, interrupt lifecycle, replay or backing failures remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

Implementation is in progress on `milestone/versioned-full-controller-virtio-blk-schema`.

## Scope boundary

This milestone deliberately does **not** serialize raw file descriptors or live ioeventfd/irqfd registrations, checkpoint an in-flight virtio request, claim cross-host migration compatibility, recreate PCI topology, add a second device, add SMP-wide migration transactions, provide external-storage durability/crash-consistency semantics, or make performance/downtime claims.

The serialized backing is the repository's bounded deterministic in-memory virtio-blk model. It is not a claim that arbitrary host files, block devices or external storage can be checkpointed consistently.

## Promotion rule

After the versioned full-controller + virtio-blk schema is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this one-device schema phase rather than farming alternate BAR values, backing patterns or corruption offsets.

The next architecture audit should prefer a genuinely larger ownership boundary. Strong candidates are composing the versioned VM/device checkpoint with reconstructable host-registration specifications so serialized state can rebuild fd-free acceleration around restore, or extending the versioned transaction to a bounded multi-device or multi-vCPU ownership set only when quiescence, dependency ordering and external-state semantics can be proven coherently. Cross-host/live migration, performance/downtime and external-storage crash consistency remain separate later frontiers.
