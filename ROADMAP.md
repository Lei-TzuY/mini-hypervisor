# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `92b0ccdd797e1d342e60b86c916e5301acf0f004` through PR #138 (`Encode page and vCPU checkpoints with a versioned schema`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; and a canonical v1 byte schema for bounded page+VCPU checkpoint state.

PR #137 seals the atomic full-controller + one-virtio-blk same-process checkpoint transaction. PR #138 then seals the first explicit serialized ownership boundary: canonical little-endian v1 page+VCPU checkpoint bytes with fail-closed compatibility/semantic validation, host-MSR revalidation during materialization, and real-KVM decode/restore/resume proof. Exact merged-main ordinary CI and all 39 applicable permanent hosted-KVM workflows are green on `92b0ccdd797e1d342e60b86c916e5301acf0f004`.

The v1 page+VCPU schema phase is sealed. Do not farm alternate corruption offsets, page orders or equivalent field encodings merely to extend the phase number.

## Selected milestone — reconstruct fd-free host registrations after checkpoint restore

The next ownership boundary is host-side KVM registration state that must be recreated rather than serialized. The sealed full-controller + virtio-blk checkpoint intentionally excludes raw file descriptors and external registration lifetimes. This milestone models the recreatable semantics of the virtio-blk queue doorbell and completion interrupt as an fd-free descriptor, then proves two fresh live registration generations around checkpoint restore.

Acceptance contract:

- preserve exact base `92b0ccdd797e1d342e60b86c916e5301acf0f004`, ordinary CI, Rust 1.74 shipped-target MSRV and every applicable permanent hosted-KVM workflow;
- the checkpoint descriptor owns only doorbell GPA, doorbell width, datamatch and GSI; no raw fd or process-local descriptor integer is captured;
- reject unsupported ioeventfd widths, datamatch values that do not fit the selected width and overflowing doorbell ranges before registration;
- reconstruction requires `KVM_CAP_IOEVENTFD` and `KVM_CAP_IRQFD`, creates fresh eventfds, assigns irqfd before ioeventfd, and rolls the irqfd assignment back if ioeventfd assignment fails;
- deassignment attempts both live registrations before returning a cleanup error; cleanup failures remain fatal;
- the mutation request must use reconstructed ioeventfd for the BAR+0x100 queue-0 notify and reconstructed irqfd for GSI0 completion; the queue-notify write must not surface as a userspace `KVM_EXIT_MMIO` device event;
- explicitly deassign the first live registration generation before machine/controller/device comparison and restore;
- after exact restore, create a second fresh live registration generation from the same fd-free semantic descriptor; freshness is proven by lifecycle and behavior, never by comparing raw fd numbers;
- mutation and replay each require exactly one ioeventfd doorbell event, one irqfd completion signal, exact guest proof `NIARD`, queue/data continuity and explicit cleanup;
- fresh verification before restore must report owned page, VCPU, master PIC, slave PIC, IOAPIC, LAPIC and virtio-blk device all non-exact; post-restore comparison must report all exact before the replay generation is created;
- replay must advance queue indices from restored `0/0` to `1/1`, preserve deterministic backing/readback equality and end with architectural RFLAGS bit 1 and IF set;
- KVM-aware integration and a dedicated permanent hosted-KVM reconstruction workflow must independently validate descriptor semantics, registration lifecycle, no-userspace-MMIO notify, interrupt completion, exact restore and replay continuity;
- `ROADMAP.md` is included in the dedicated workflow path filter so the documentation-synchronized candidate reruns the real-KVM reconstruction proof;
- capability, registration, rollback, deassignment, transport ownership, comparison, interrupt, queue, backing/readback or replay failures remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

Implementation is in progress on `milestone/host-registration-reconstruction` through PR #139. The branch has been rebuilt on the integrated versioned-schema main rather than merging a stale tree. Its first prior CI attempt never reached semantic verification because `src/kvm/sys.rs` contained a literal escaped newline between include directives; that construction error is corrected in the rebased candidate. Full ordinary CI and every applicable permanent hosted-KVM workflow must now rerun before any integration decision.

## Scope boundary

This milestone deliberately does **not** serialize raw file descriptors, encode host registrations into the v1 page+VCPU schema, claim cross-host migration compatibility, checkpoint an in-flight request, recreate PCI topology, add multi-device/SMP checkpoint transactions, provide external-storage durability semantics, or make performance/downtime claims.

## Promotion rule

After host-registration reconstruction is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal the single-device registration lifecycle rather than farming alternate fd values, BAR aliases or GSIs.

The next architecture audit should either extend the explicit versioned schema to the already-sealed controller/device aggregate with compatibility validation, or broaden to a genuinely larger atomic ownership boundary only when quiescence and restore ordering can be proven coherently. Cross-host/live migration, multi-device migration, performance/downtime and external-storage crash consistency remain separate later frontiers.
