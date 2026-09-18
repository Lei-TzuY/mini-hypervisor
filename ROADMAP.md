# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO execution, long-mode virtual-MMIO composition, direct long-mode interrupt delivery, one in-kernel x86 irqchip/GSI route, MMIO-device interrupt delivery, stateful device-owned level-interrupt lifecycle, bounded two-device MMIO registration/mapping, two independently routed MMIO level-interrupt sources, and one genuinely host-driven asynchronous timer wakeup.

The asynchronous timer phase is integrated at commit `e2c0f1c7686e39a31949e038d1d6ba7d4bf70746` through PR #86. Exact merged-main CI #389 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all ten earlier strict real-KVM gates, and the eleventh strict async-timer gate. Its executable proof keeps IF clear through readiness `R` and arm barrier `A`, uses adjacent `sti; hlt`, receives a host-worker GSI0 edge through the existing PIC/LAPIC ExtINT route, emits handler byte `T`, resumes the halted mainline with `W`, and reaches terminal userspace barrier `D`. Exact proof is `RATWD`; arm RFLAGS has architectural bit 1 set with IF clear, while completion has bit 1 and IF set.

That direct worker-ioctl timer phase is sealed. Do not farm fixed delay variants, more one-shot timer instances, or additional direct `KVM_IRQ_LINE` workers merely to extend the phase number.

## Selected milestone — KVM irqfd-backed asynchronous timer delivery

The host-driven one-shot timer phase is integrated on `main` at commit `e2c0f1c7686e39a31949e038d1d6ba7d4bf70746` through PR #86. Exact merged-main CI #389 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, the ten earlier strict real-KVM gates, and the eleventh strict asynchronous timer gate. Its executable proof is exact `RATWD`: arm barrier `A` observes IF clear, the adjacent `sti; hlt` handoff is woken by a worker-thread GSI0 edge, vector `0x40` handler emits `T`, resumed main emits `W`, and terminal barrier `D` observes IF enabled.

That direct worker-thread VM-ioctl timer phase is sealed. Do not farm additional fixed delays or clone the same `KVM_IRQ_LINE` transport.

The next boundary is kernel-accelerated asynchronous event delivery through `KVM_IRQFD`. The guest, PIC/LAPIC ExtINT state, race-safe `cli` arm barrier, adjacent `sti; hlt`, handler, watchdog and exact `RATWD` verifier remain unchanged. Only the host transport changes: userspace registers an eventfd with GSI0 through `KVM_IRQFD`; the timer worker owns only a duplicated eventfd and signals one u64 event, never a VM fd or guest RAM.

Acceptance contract:

- preserve all eleven integrated strict real-KVM gates and all existing MSRV, CPU-policy, MMIO, interrupt, diagnostics and safety contracts;
- require `KVM_CAP_IRQFD` as a hard capability for this executable path;
- implement and layout-test the exact Linux `struct kvm_irqfd` ABI, including assignment and `KVM_IRQFD_FLAG_DEASSIGN`;
- create the eventfd with close-on-exec/nonblocking ownership, duplicate only the eventfd for the timer worker, and signal it with one exact u64 write;
- bind eventfd→GSI0 before entering the potentially blocking `sti; hlt` handoff;
- the irqfd timer worker must not call `KVM_IRQ_LINE`, own a KVM VM fd, or alias guest RAM;
- explicitly deassign the irqfd registration on every non-hanging completion path before accepting proof;
- retain the direct-GSI watchdog only as fail-closed anti-hang protection; any watchdog intervention is a hard failure and cannot manufacture accepted proof;
- reuse exact guest proof `RATWD`, requiring A-barrier bit1 with IF clear and completion bit1 with IF set;
- preserve software-enabled LAPIC SPIV and unmasked ExtINT LINT0;
- KVM-aware integration must validate capability, GSI/vector, LAPIC state, RFLAGS and all five exact debug-port exits;
- stable CI must retain the eleven integrated strict gates and add an independent twelfth hosted-KVM irqfd gate requiring capability text, GSI0, vector `0x40`, semantic LAPIC state, exact arm/completion flags and proof `[82, 65, 84, 87, 68]`;
- capability, eventfd, irqfd assignment/deassignment, worker, watchdog, guest proof or controller-state failures remain hard failures and must not be skipped or retried into success.

## Scope boundary

This milestone deliberately does **not** add irqfd resample/level semantics, `KVM_CAP_IRQFD_RESAMPLE`, ioeventfd, arbitrary GSI routing, IOAPIC/MSI/MSI-X, periodic timers, PIT/HPET/APIC timer emulation, TSC-deadline, a generic scheduler, PCI/virtio, SMP, DMA/IOMMU, migration, snapshots, latency benchmarks, or guest-memory cross-thread sharing.

## Promotion rule

After irqfd timer delivery is integrated and exact merged-`main` CI is green, seal the first irqfd acceleration proof rather than multiplying eventfds or fixed GSIs.

The next architecture audit should choose another materially new interaction boundary. Strong candidates are a minimal `KVM_IOEVENTFD`-backed guest doorbell path only if it closes an executable device event/interrupt round trip, or a minimal PCI/virtio transport that introduces real discovery/configuration semantics. IOAPIC/MSI, SMP, DMA/IOMMU, migration, irqfd resample semantics, and performance work remain separate higher-order frontiers.
