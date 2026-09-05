# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO execution, long-mode virtual-MMIO composition, direct long-mode interrupt delivery, one in-kernel x86 irqchip/GSI route, MMIO-device interrupt delivery, stateful device-owned level-interrupt lifecycle, bounded two-device MMIO registration/mapping, and two independent MMIO level-interrupt sources routed through distinct legacy-PIC GSIs/vectors.

The dual-source routing phase is integrated at commit `22e6706483f4c4bcc07386115df362fc83cb169e` through PR #85. Exact merged-main CI #384 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all nine earlier strict real-KVM gates, and the tenth strict dual-source routing gate. Its executable proof retains source identity through two registered MMIO devices and routes GPA `0x10000000` through GSI0/vector `0x40` and GPA `0x10001000` through GSI1/vector `0x41`; two distinct handlers produce exact proof `A0SCMB1TEND` while both level lifecycles complete STATUS/ACK ownership and return to one guest mainline.

That two-fixed-source legacy-PIC phase is sealed. Do not farm GSI2/GSI3 clones, a third identical MMIO source, or more fixed-address variants merely to extend the phase number.

## Selected milestone — host-driven one-shot timer interrupt wakeup

The next architecture boundary is asynchronous host event delivery. Every integrated device interrupt so far is ultimately initiated by guest execution reaching a userspace-visible MMIO command or by userspace synchronously pulsing a line between KVM runs. This milestone adds a genuinely different interaction pattern: an independent host worker owns only a duplicated KVM VM fd, waits for a one-shot timer, and drives GSI0 while the boot vCPU proceeds through a race-safe `sti; hlt` handoff.

The milestone deliberately reuses the already-proven in-kernel legacy PIC and LAPIC ExtINT path. It does not claim PIT, HPET, local-APIC timer, TSC-deadline, realtime scheduling, or a generic timer framework.

Acceptance contract:

- preserve all ten integrated strict real-KVM gates and every existing long-mode, ELF64, MMIO, interrupt, snapshot, CPU-policy, diagnostic, and MSRV contract;
- do not make `Vm`, `GuestMemory`, or the guest RAM mapping Send/Sync merely to obtain a worker thread;
- derive a worker-safe IRQ-line handle by duplicating only the KVM VM file descriptor; the worker must not own, dereference, or alias guest RAM;
- wrap every successful duplicate fd immediately in `OwnedFd`, surface duplication/ioctl failures as hard errors, and prove the worker handle is `Send` through the type system;
- validate the duplicated IRQ-line descriptors with an inert deassert before the guest enters the potentially blocking HLT path, so setup failures remain synchronous;
- deterministic guest setup programs the existing legacy master/slave PIC mapping, unmasks only IRQ0, keeps IF clear, emits readiness `R`, then emits explicit timer-arm barrier `A` while IF is still clear;
- the next two guest instructions must be adjacent `sti; hlt`; x86 STI interrupt shadow is the correctness mechanism that closes the race if the host edge becomes pending immediately before KVM executes HLT;
- after the one-shot timer edge, vector `0x40` handler emits `T`, sends master-PIC EOI, and `iretq`; resumed mainline emits `W`, then terminal userspace barrier `D`;
- exact debug-port proof is `RATWD` across exactly five byte-wide exits, in that order;
- the A-barrier register snapshot must have architectural RFLAGS bit 1 set and IF clear; the completion snapshot after `D` must have bit 1 and IF set;
- LAPIC SPIV remains software-enabled and LINT0 remains unmasked ExtINT;
- timer scheduling delay is only a trigger mechanism and is not a latency benchmark, deadline guarantee, performance assertion, or correctness threshold;
- a separate fail-closed watchdog may inject a fallback GSI solely to prevent a broken timer path from wedging CI indefinitely inside KVM_RUN; if that watchdog ever fires, the execution must fail even when later guest bytes happen to match the nominal proof;
- timer-worker panic, timer ioctl failure, watchdog intervention, watchdog failure, unexpected VM exit, wrong byte order, wrong PIC/LAPIC state, or wrong RFLAGS must remain hard failures;
- KVM-aware integration must validate GSI/vector, semantic LAPIC state, the IF-clear arm point, IF-enabled completion, all five debug-port exits, and exact `RATWD` proof;
- stable CI must retain the ten integrated strict real-KVM gates and add an independent eleventh hosted-KVM gate for the async timer executable.

## Scope boundary

This milestone deliberately does **not** add:

- periodic or programmable timers, PIT/HPET emulation, local-APIC timer programming, TSC-deadline, timer wheels, or a general scheduler;
- realtime/wall-clock latency guarantees, controlled performance benchmarks, or host scheduling claims;
- arbitrary `KVM_SET_GSI_ROUTING`, IOAPIC programming, MSI/MSI-X, x2APIC, shared GSI arbitration, or slave-PIC expansion;
- `eventfd`, `irqfd`, `ioeventfd`, or another acceleration path;
- PCI/PCIe configuration space, BARs, virtio transport, DMA, IOMMU, or device hotplug;
- guest-memory sharing with the timer thread, multi-vCPU delivery, SMP, migration, resumable execution, or whole-VM snapshots.

## Promotion rule

After the one-shot host timer is integrated and exact merged-`main` CI is green, seal the single asynchronous wakeup proof rather than multiplying fixed timer delays or timer instances.

The next architecture audit should choose another materially new frontier. Strong candidates are a minimal reusable event/interrupt scheduling abstraction only if it supports a second executable source without duplicating this fixture, `eventfd`/`irqfd` acceleration backed by comparative behavior evidence, or a minimal PCI/virtio transport that introduces real discovery/configuration semantics. IOAPIC/MSI, SMP, DMA/IOMMU, and migration remain separate higher-order phases.