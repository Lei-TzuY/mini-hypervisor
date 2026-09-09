# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `24adc1ed335c3f53f6a72519ed6c0225da0d25a3` through PR #131 (`Keep remaining strict KVM timeouts independent of cold builds`). Exact ordinary CI and all permanent hosted-KVM workflows for that integrated frontier were settled successfully before the controller-checkpoint branch was created.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest-owned task scheduling/wait ownership; one-page and bounded multi-page VCPU checkpoints; scheduler/wait checkpoint composition; and one coordinated two-VCPU stop-the-world checkpoint.

PR #129 seals the coordinated two-VCPU checkpoint frontier at `72fc26574a6f6d90e37f41689b5e8d10511ff3d6`. It captures one canonical shared page set plus exactly two VCPU snapshots only after both deterministic quiescent boundaries, proves independent page/VCPU divergence, restores in canonical order and resumes both VCPUs with exact real-KVM proofs. PRs #130 and #131 add no guest-visible capability; they move remaining cold builds outside unchanged strict 30-second KVM execution budgets so compilation cannot masquerade as an execution timeout.

The fixed two-VCPU page/VCPU checkpoint phase is sealed. Do not farm VCPU2/VCPU3 clones, extra marker pages or alternate fixed checkpoint fixtures merely to extend the phase number.

## Selected milestone — bounded master-PIC and LAPIC controller checkpoint

The next architecture boundary is kernel-owned interrupt-controller state. Existing checkpoint machinery already owns a bounded page set and canonical VCPU state; existing controller-backed execution already owns an in-kernel x86 irqchip and boot-vCPU LAPIC. This milestone composes those layers without cloning page/VCPU snapshot logic: one owned page, one canonical VCPU snapshot, the in-kernel master PIC and the full boot-vCPU LAPIC become one bounded checkpoint that must survive deliberate four-way corruption and then resume real interrupt delivery.

The first implementation attempt exposed an important quiescence invariant. An in-kernel LAPIC may keep guest HLT inside KVM as a non-runnable VCPU instead of guaranteeing `KVM_EXIT_HLT`; using HLT as the controller-checkpoint capture boundary therefore hung the permanent hosted-KVM proof. The corrected milestone uses an already-integrated guest-debug primitive instead: after PIC setup and writing marker `A`, the guest emits a capture-arm byte `C`; userspace services that PIO, enables KVM single-step, re-enters once so the serviceable PIO is completed, receives `KVM_EXIT_DEBUG`, disables single-step and captures only from that non-serviceable IF-clear boundary. Real KVM establishes the exact capture state as RIP `0x10031`, RFLAGS `0x2`.

Acceptance contract:

- preserve exact base `24adc1ed335c3f53f6a72519ed6c0225da0d25a3`, Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow;
- reuse `BoundedVcpuPageSetCheckpoint` for the owned page plus canonical VCPU state; do not fork the existing page/VCPU capture or restore pipeline;
- model Linux x86 `kvm_irqchip` exactly for the master PIC: 520-byte request, 16-byte master-PIC state, GET `0xC208_AE62`, SET `0x8208_AE63`; union padding is not checkpoint state;
- capture the complete existing 0x400-byte boot-VCPU LAPIC state through the established KVM LAPIC UAPI;
- use the explicit capture-arm PIO plus KVM single-step completion handoff as the quiescent boundary; capture is accepted only on `KVM_EXIT_DEBUG` at RIP `0x10031` with architectural RFLAGS bit 1 set and IF clear;
- disable guest single-step again before any checkpoint state is captured; a run-control failure or unexpected exit remains a hard failure;
- capture page `0x30000` containing marker `A`, the canonical VCPU snapshot, master PIC IMR `0xfe`, software-enabled LAPIC SPIV and unmasked LINT0 ExtINT;
- deliberately corrupt the owned page, VCPU state, master-PIC IMR and LAPIC LINT0 mask; fresh verification must report all four components non-exact independently;
- restore page/VCPU first and require that layer exact before controller mutation; then restore master PIC, then LAPIC, followed by a fresh four-component verification before any resumed guest execution;
- after exact restore, the guest executes the preserved NOP then `sti; nop`, loads restored marker `A`, emits armed barrier `A`, receives userspace GSI0 through vector `0x40`, executes handler output `I` + PIC EOI + `iretq`, resumes main output `M`, then emits completion barrier `D`;
- exact executable proof remains `AIMD` across four byte-wide debug-port exits; armed and completion RFLAGS must both retain bit 1 and IF;
- the KVM-aware integration independently validates capture RIP/RFLAGS, four mismatch→exact transitions, PIC/LAPIC semantics, all proof exits and resumed interrupt delivery;
- the permanent hosted-KVM workflow builds the proof outside its 30-second execution timeout and requires the same capture, restore and `AIMD` evidence; `/dev/kvm` absence, timeout, changed expected values or skipped verification are not successful outcomes;
- `ROADMAP.md` is part of that workflow's path filter so the final documentation-synchronized candidate must re-run the controller proof rather than inheriting evidence from an earlier code SHA.

Implementation is in progress on `milestone/controller-checkpoint` through draft PR #132. The pre-documentation exact code candidate `055ffed2fabb67be1d3cebb2d270c5c213e06ccf` passed the corrected hosted-KVM controller proof with capture `KVM_EXIT_DEBUG @ 0x10031 / RFLAGS 0x2`, corruption `page=false vcpu=false master-pic=false lapic=false`, restore `page=true vcpu=true master-pic=true lapic=true`, PIC IMR `0xfe`, LAPIC SPIV `0x1ff`, LINT0 `0x700`, armed/completion RFLAGS `0x202` and proof `AIMD`. It also settled with no failed, queued or in-progress permanent workflows. This evidence is not the final merge certificate: the documentation-synchronized head must independently settle green before integration.

## Scope boundary

This milestone deliberately does **not** add:

- slave-PIC or IOAPIC checkpoint state, arbitrary irqchip routing or a whole-irqchip serialization format;
- irqfd/eventfd/ioeventfd registration checkpointing, replay or host-fd migration;
- PCI/virtio device-state checkpointing, DMA/IOMMU state or device inflight replay;
- capture from arbitrary in-flight PIO/MMIO service exits;
- SMP controller checkpointing, cross-VCPU pending interrupt replay or a whole-VM migration transaction;
- migration serialization/versioning, cross-host compatibility, pre-copy/post-copy or crash-consistency claims;
- performance, downtime or portability claims beyond the exact hosted-KVM evidence.

## Promotion rule

After the bounded master-PIC/LAPIC controller checkpoint is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this fixed controller proof rather than farming alternate PIC masks, vectors, marker pages or LAPIC bits.

The next architecture audit should promote to a materially broader state-ownership frontier. The preferred candidate is completion of bounded in-kernel irqchip checkpoint coverage — slave PIC plus IOAPIC composed with the existing master-PIC/LAPIC/page/VCPU checkpoint — only if the Linux KVM UAPI, restore ordering, quiescence boundary and resumed interrupt proof can all be represented explicitly and demonstrated on real KVM. irqfd/eventfd registrations, PCI/virtio device state, in-flight service-exit replay and migration serialization remain separate higher-order frontiers and must not be claimed by controller-state coverage alone.
