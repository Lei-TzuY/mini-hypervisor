# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 loading and non-identity virtual mapping, bidirectional MMIO, multi-device MMIO registration/mapping, direct and controller-backed interrupt delivery, stateful MMIO level-interrupt lifecycles, two independent legacy-PIC interrupt sources, and one host-driven asynchronous timer wakeup.

The host-driven timer phase is integrated at commit `e2c0f1c7686e39a31949e038d1d6ba7d4bf70746` through PR #86. Exact merged-main CI #389 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all ten earlier strict real-KVM gates, and the eleventh strict async-timer gate. Its deterministic guest keeps IF clear through arm barrier `A`, executes adjacent `sti; hlt`, receives GSI0/vector `0x40` from a host worker that owns only a duplicated VM fd, emits handler byte `T`, resumes with `W`, and completes exact proof `RATWD`. The watchdog is fail-closed and cannot manufacture a passing proof.

That direct-VM-ioctl one-shot timer phase is sealed. Do not farm more fixed delays, duplicate timer instances, or additional direct `KVM_IRQ_LINE` variants.

## Selected milestone — accelerated one-shot timer delivery through KVM irqfd

The next boundary is event-delivery transport. The integrated asynchronous timer already proves that a host worker can wake a halted guest through the existing PIC/LAPIC ExtINT route. This milestone keeps the same guest, handler, race-safe `cli → A → sti; hlt` handoff, watchdog and exact `RATWD` proof, but replaces worker-thread VM ioctls with Linux KVM's irqfd/eventfd acceleration path.

Acceptance contract:

- preserve all eleven integrated strict real-KVM gates and every existing long-mode, ELF64, MMIO, interrupt, snapshot, CPU-policy, diagnostic and Rust 1.74 MSRV contract;
- require `KVM_CAP_IRQFD` as a hard capability for the irqfd executable and strict hosted-KVM gate; capability absence is not a permitted skip;
- implement the exact 32-byte Linux `struct kvm_irqfd` ABI, `KVM_IRQFD` request, and `KVM_IRQFD_FLAG_DEASSIGN`, with deterministic layout/flag regressions;
- create the event source with `eventfd(EFD_CLOEXEC | EFD_NONBLOCK)`, immediately own successful descriptors, and duplicate only the eventfd for the timer worker;
- the irqfd worker must never own a VM fd, call `KVM_IRQ_LINE`, or alias guest memory; it signals exactly one u64 eventfd increment;
- bind the eventfd to GSI0 before entering the potentially blocking `sti; hlt` handoff and preserve the existing in-kernel irqchip, PIC mapping, software-enabled LAPIC SPIV and unmasked ExtINT LINT0;
- explicitly deassign the irqfd registration on every non-hanging completion path before proof acceptance; deassign failure is a hard failure;
- retain the direct-GSI watchdog only as anti-hang protection; any watchdog intervention is a hard failure and cannot satisfy the irqfd proof;
- reuse the deterministic guest proof exactly: readiness `R`, arm barrier `A` with IF clear, irqfd-delivered handler `T`, resumed mainline `W`, terminal barrier `D`;
- require exact `RATWD` proof across five byte-wide debug-port exits, GSI0/vector `0x40`, armed RFLAGS bit1 with IF clear, completion RFLAGS bit1 with IF set, semantic LAPIC SPIV/LINT0 state, and no unexpected exits;
- KVM-aware integration may skip only when `/dev/kvm` is unavailable or permission denied; missing `KVM_CAP_IRQFD`, irqfd assignment/deassignment failure, eventfd failure, worker panic, watchdog intervention, wrong proof or wrong architectural state remain hard failures;
- stable CI must retain all eleven integrated strict real-KVM gates and add an independent twelfth irqfd timer gate that runs the irqfd executable and requires the exact capability, controller-state, RFLAGS and `RATWD` evidence.

## Scope boundary

This milestone deliberately does **not** add irqfd resample/level semantics, `KVM_CAP_IRQFD_RESAMPLE`, ioeventfd, arbitrary GSI routing, IOAPIC/MSI/MSI-X, periodic timers, PIT/HPET/APIC timer emulation, TSC-deadline, scheduler frameworks, PCI/virtio, SMP, DMA/IOMMU, migration, or performance/latency claims.

## Promotion rule

After irqfd timer delivery is integrated and exact merged-`main` CI is green, seal the two-transport one-shot timer proof rather than multiplying irqfd registrations or fixed delays.

The next architecture audit should move to a materially different frontier. Strong candidates are ioeventfd-backed MMIO notification with an executable guest/device interaction, a minimal PCI/virtio discovery/configuration surface, or a higher-order interrupt/controller phase only when it adds new observable behavior rather than another fixed routing variant. SMP, DMA/IOMMU and migration remain separate frontiers.
