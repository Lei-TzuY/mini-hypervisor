# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `dc32094168fe3d0810a944f6e2d02e2f2a731831` through PR #139 (`Reconstruct host registrations after checkpoint restore`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; a canonical v1 byte schema for page+VCPU checkpoint state; and reconstruction of fd-free ioeventfd/irqfd host registrations around exact restore.

PR #138 sealed canonical little-endian v1 page+VCPU checkpoint bytes with fail-closed compatibility/semantic validation and host-MSR revalidation during materialization. PR #139 then sealed the first recreatable host-registration boundary: raw fds remain outside serialized ownership, while doorbell/GSI semantics rebuild two fresh ioeventfd/irqfd generations around exact restore and replay. Exact merged-main ordinary CI and every applicable permanent hosted-KVM workflow are green on `dc32094168fe3d0810a944f6e2d02e2f2a731831`.

The single-device host-registration reconstruction phase is sealed. Do not farm alternate fd values, BAR aliases, equivalent GSI assignments or registration generations merely to extend the phase number.

## Selected milestone — versioned full-controller checkpoint schema

The next ownership boundary is the already-integrated full in-kernel controller aggregate. The live controller checkpoint captures page+VCPU state together with master PIC, slave PIC, IOAPIC and LAPIC state, but only the page+VCPU subset has an explicit canonical byte schema. This milestone extends the serialized compatibility boundary to the controller aggregate without encoding raw Linux `struct kvm_irqchip` padding or host registration lifetimes.

Acceptance contract:

- preserve exact base `dc32094168fe3d0810a944f6e2d02e2f2a731831`, ordinary CI, Rust 1.74 shipped-target MSRV and every applicable permanent hosted-KVM workflow;
- define a canonical little-endian v1 full-controller envelope with explicit magic, version, architecture, fixed header length, total length, nested guest length, zero flags and zero reserved fields;
- reuse the sealed v1 page+VCPU payload as the nested guest state rather than duplicating that schema;
- encode exactly the semantic 16-byte master-PIC state, 16-byte slave-PIC state, 216-byte IOAPIC state and 1024-byte LAPIC register file; Linux `struct kvm_irqchip` outer padding is not serialized;
- reject wrong magic/version/architecture/header/total/nested lengths, non-zero flags/reserved fields, nested-schema corruption, length overflow and non-zero IOAPIC pad fail-closed;
- materialization must re-run the nested schema's current-host MSR compatibility validation before reconstructing `BoundedFullControllerCheckpoint`;
- preserve existing guest-first then controller restore ordering and exact post-restore comparison;
- versioned execution must drop the original process-local checkpoint before decode/materialize, require canonical decode→re-encode identity, then run the same corruption→restore→slave-PIC→IOAPIC→completion proof as the direct transport;
- deterministic unit coverage must include canonical semantic roundtrip, envelope/header corruption, nested corruption, IOAPIC pad validation, host-MSR revalidation and truncation;
- KVM-aware integration and a dedicated permanent hosted-KVM workflow must independently validate schema metadata, canonical roundtrip, full six-component corruption/restoration, controller semantics, restored interrupt routes and exact proof `ASBJMD`;
- `ROADMAP.md` is included in the dedicated workflow path filter so the documentation-synchronized candidate reruns the executable proof;
- schema, compatibility, semantic controller, restore-ordering, interrupt-route or proof failures remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

Implementation is in progress on `milestone/versioned-full-controller-checkpoint-schema` through PR #140.

## Scope boundary

This milestone deliberately does **not** serialize raw file descriptors or live ioeventfd/irqfd registrations, claim cross-host migration compatibility, checkpoint an in-flight request, recreate PCI topology, extend the schema to virtio-blk device/backing state, add multi-device/SMP migration transactions, provide external-storage durability semantics, or make performance/downtime claims.

## Promotion rule

After the versioned full-controller schema is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this controller-envelope phase rather than farming corruption offsets or alternate encodings.

The next architecture audit should either extend the explicit versioned schema to the already-sealed full-controller + one-virtio-blk atomic checkpoint, including device/backing compatibility validation, or broaden to a genuinely larger ownership boundary only when quiescence, external-state semantics and restore ordering can be proven coherently. Cross-host/live migration, multi-device migration, performance/downtime and external-storage crash consistency remain separate later frontiers.
