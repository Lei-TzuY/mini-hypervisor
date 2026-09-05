# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO device execution, bounded long-mode virtual-MMIO composition, bounded direct long-mode interrupt delivery, and one bounded controller-backed GSI0 route through KVM's in-kernel x86 irqchip.

The controller-backed path creates the irqchip before vCPU creation, preserves unrelated LAPIC state while configuring BSP SPIV/LINT0 for legacy ExtINT delivery, initializes the guest PIC, pulses GSI0 through `KVM_IRQ_LINE`, enters vector `0x40`, sends PIC EOI, executes `IRETQ`, resumes main code, and terminates userspace observation at an explicit completion barrier rather than relying on `KVM_EXIT_HLT` with an in-kernel LAPIC.

Merged-main CI requires real-KVM executable evidence for seven established layers through the currently integrated controller phase: long-mode, ELF64, real-mode MMIO, long-mode virtual-MMIO, direct-vector interrupt delivery, and irqchip/GSI delivery are strict gates; the selected device-generated interrupt slice adds the next independent gate only after its exact candidate is proven.

## Selected milestone — bounded MMIO device-generated interrupt composition

This milestone connects the existing userspace MMIO data plane to the integrated controller-backed interrupt plane. One accepted write to the fixed byte-wide virtual MMIO device must create one owned userspace device event; that event is consumed exactly once and is the sole authority for pulsing the established fixed GSI0 route.

Acceptance contract:

- preserve all existing long-mode, ELF64, MMIO, virtual-MMIO, direct-interrupt, irqchip/GSI, CPU-policy, snapshot, diagnostic, and strict real-KVM contracts;
- extend the existing byte device with an explicit interrupting mode rather than changing ordinary MMIO device behavior globally;
- publish `InterruptRequested` only after a valid one-byte write is accepted by the interrupting device; reads, unknown addresses, unsupported widths, and malformed write payloads must not create an interrupt event;
- own the event in userspace device state until consumed; one successful write yields at most one consumable event, a second consume returns `None`, and a later valid write may create a new event;
- compose the existing virtual-MMIO page-table mapping with the existing fixed GDT/IDT interrupt layout without duplicating either subsystem or allowing one table installation to overwrite the other;
- create the in-kernel irqchip before the vCPU, retain the established PIC remap/unmask contract, and require semantic LAPIC SPIV/LINT0 readback before executable proof continues;
- require the first userspace-visible device exit to be an exact one-byte MMIO write of `W` to translated device GPA `0x10000000`;
- do **not** interpret `KVM_GET_REGS` taken directly from the in-flight `KVM_EXIT_MMIO` as completed architectural state and do not pulse GSI while that MMIO operation is still pending;
- after servicing the write and creating the pending device event, re-enter KVM until the guest emits `A`; observing this barrier proves the preceding MMIO operation was completed on a later `KVM_RUN` before interrupt delivery is armed;
- only after the `A` barrier, require architectural RFLAGS bit 1 plus IF, consume exactly one `InterruptRequested` event, verify a second consume is empty, and then pulse fixed GSI0 as level 1 followed by level 0;
- after the pulse, require the next proof byte to be handler byte `I`; seeing resumed-main output first is failure rather than delayed-success tolerance;
- the handler must send non-specific EOI to the master PIC and execute `IRETQ`; resumed main must emit `M`, followed by `D` as the userspace completion barrier;
- exact host-visible proof is therefore one MMIO write `W`, exactly one consumed device event, GSI0/vector `0x40`, LAPIC ExtINT readback, and debug proof `AIMD` with IF set at the armed and completion observations;
- do not require the safety-fallback HLT as terminal evidence because the integrated in-kernel LAPIC path does not promise a userspace HLT exit;
- stable CI must retain all six previously integrated strict real-KVM proofs and add an independent MMIO-device-interrupt gate. KVM-aware integration must also assert MMIO direction/address/length/payload, exact debug-port metadata, one-shot event count, LAPIC state, and RFLAGS contract;
- any capability, irqchip creation, LAPIC read/write/readback, MMIO decode/service, event ownership, GSI assert/deassert, handler-order, proof, or architectural-state failure remains a hard failure and must not be skipped, retried into success, or weakened into a best-effort claim.

## Scope boundary

This milestone deliberately does **not** add:

- a general device-event queue, interrupt coalescing policy, multiple pending events, or reusable interrupt scheduler;
- caller-defined `KVM_SET_GSI_ROUTING` tables, arbitrary GSIs, IOAPIC redirection ownership, MSI/MSI-X, or PCI interrupt delivery;
- PIT, HPET, LAPIC timer, TSC-deadline timer, periodic scheduling, or timer-driven interrupts;
- x2APIC exposure, SMP, cross-vCPU routing, or multiple vCPUs;
- irqfd/ioeventfd/eventfd acceleration;
- a general MMIO range/device registry, register bank, virtio transport, DMA, or IOMMU model;
- multiple RAM slots, memory hotplug, whole-VM snapshots, migration, or resumable execution;
- arbitrary caller-supplied GDT/IDT/page-table layouts or guest-controlled descriptor-table construction.

## Promotion rule

After MMIO device-generated interrupt composition is integrated and exact merged-`main` CI is green, seal the one-shot device-event-to-controller path and perform another architecture/integration audit. Do not farm additional byte values, fixed GSI numbers, or duplicate proof fixtures.

The next phase should promote architecture depth rather than repeat this path: prefer a coherent reusable device/register model or a stronger interrupt-routing/device abstraction only when it has a second real consumer and an executable cross-layer proof. General routing, timers, irqfd acceleration, PCI/virtio, SMP, or migration remain separate milestones and must earn their own implementation plus evidence.
