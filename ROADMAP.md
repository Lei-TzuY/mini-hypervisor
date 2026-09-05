# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO device execution, bounded long-mode virtual-MMIO composition, bounded direct long-mode interrupt delivery, one bounded controller-backed GSI0 route through KVM's in-kernel x86 irqchip, and one bounded MMIO-device-generated interrupt path.

The integrated MMIO-device interrupt path owns one userspace `InterruptRequested` event per accepted byte write, waits for a later `KVM_RUN` completion barrier before consuming that event, pulses fixed GSI0 through the established PIC/LAPIC path, enters vector `0x40`, sends PIC EOI, executes `IRETQ`, resumes main code, and terminates userspace observation at an explicit completion barrier. Exact merged-main CI proves this path with `AIMD` and retains the earlier long-mode, ELF64, MMIO, virtual-MMIO, direct-interrupt, and irqchip/GSI gates.

Merged-main CI therefore requires seven strict real-KVM executable gates through the one-shot device-event phase. The selected milestone below adds an eighth gate only after exact candidate evidence proves a device-owned level interrupt lifecycle end to end.

## Selected milestone — device-owned MMIO level interrupt lifecycle

This milestone promotes the one-shot edge-style device event into a stateful device/register lifecycle. A command write makes the device pending and authorizes one GSI-level assertion request; the interrupt handler must observe pending status through MMIO, acknowledge the device through a separate MMIO register, and only after a later completion barrier may userspace consume one deassert request and lower the GSI line before PIC EOI/`IRETQ` completes.

Acceptance contract:

- preserve every existing long-mode, ELF64, MMIO, virtual-MMIO, direct-interrupt, irqchip/GSI, one-shot MMIO-device interrupt, CPU-policy, snapshot, diagnostic, and strict real-KVM contract;
- retain ordinary and one-shot interrupting byte-device semantics unchanged; add an explicit level-interrupt register mode rather than changing the existing device globally;
- expose three byte-wide registers at the established translated device GPA: COMMAND at offset `0`, STATUS at offset `1`, and ACK at offset `2`;
- a valid COMMAND write records the command and makes the device pending; invalid address/direction/width/payload accesses are hard failures and publish no line transition request;
- pending device state authorizes at most one consumable `InterruptLineAssertRequested` transition until ACK; repeated commands while already pending may update the write trace but must not create duplicate line assertions;
- STATUS is read-only and returns exactly `1` while pending and `0` after ACK; the interrupt handler, not userspace alone, must consume the STATUS response and prove it observed `1`;
- ACK is write-only; a valid ACK clears pending state and authorizes at most one consumable `InterruptLineDeassertRequested` transition while the host-side line mirror remains asserted;
- expose explicit `Vm::set_gsi_level(gsi, asserted)` through the existing `KVM_IRQ_LINE` boundary, and retain `pulse_gsi_edge` as composition of assert then deassert rather than duplicating IRQ UAPI code;
- require the first userspace-visible device exit to be exact one-byte COMMAND write `W` to GPA `0x10000000`;
- do **not** treat a serviceable `KVM_EXIT_MMIO` as completed architectural state: after COMMAND, re-enter KVM until debug byte `A` before consuming the assert request or driving GSI high;
- at `A`, require architectural RFLAGS bit 1 plus IF, consume exactly one assert request, verify a second consume is empty, then set fixed GSI0 high;
- require handler entry byte `I` before any resumed-main proof;
- require the handler STATUS access to be an exact one-byte read from GPA `0x10000001`; userspace must provide byte `1`, then re-enter KVM and require debug byte `S`, proving guest code actually compared and accepted STATUS=1 rather than merely preparing a userspace response;
- require the handler ACK access to be an exact one-byte write of `1` to GPA `0x10000002`;
- ACK remains in-flight at its `KVM_EXIT_MMIO`; userspace must re-enter KVM until debug byte `C` before consuming the deassert request or setting GSI low;
- only after `C`, consume exactly one deassert request, verify a second consume is empty, and set fixed GSI0 low; lowering the line directly at the ACK exit is forbidden;
- after the line is low, the handler sends non-specific EOI to the master PIC and executes `IRETQ`; resumed main emits `M`, followed by `D` as the userspace completion barrier;
- exact host-visible proof is therefore COMMAND/ACK writes `[W, 1]`, one assert request, one deassert request, fixed GSI0/vector `0x40`, semantic LAPIC ExtINT readback, and debug proof `AISCMD` with IF set at armed and completion observations;
- the failure path for a guest-observed STATUS other than `1` emits `F`, which is a hard proof failure rather than tolerated alternative behavior;
- do not require the safety-fallback HLT as terminal evidence because the integrated in-kernel LAPIC path does not promise a userspace HLT exit;
- stable CI must retain all seven integrated strict real-KVM gates and add an independent MMIO level-interrupt lifecycle gate; KVM-aware integration must independently validate COMMAND/STATUS/ACK MMIO metadata, line-transition counts, writes, LAPIC state, IF, and each proof byte;
- capability, irqchip, LAPIC, MMIO decode/service/response, event ownership, GSI-level transition, handler ordering, proof, or architectural-state failures remain hard failures and must not be retried, swallowed, skipped, or converted to best-effort success.

## Scope boundary

This milestone deliberately does **not** add:

- a general event queue, arbitrary interrupt priorities, multiple independently pending causes, or a reusable interrupt scheduler;
- caller-defined GSI routing tables, arbitrary GSIs, IOAPIC redirection ownership, MSI/MSI-X, or PCI interrupt delivery;
- PIT, HPET, LAPIC timer, TSC-deadline timer, periodic scheduling, or timer-driven interrupts;
- x2APIC exposure, SMP, cross-vCPU routing, or multiple vCPUs;
- irqfd/ioeventfd/eventfd acceleration;
- a general multi-device MMIO registry, virtio transport, DMA, IOMMU, or bus enumeration model;
- multiple RAM slots, memory hotplug, whole-VM snapshots, migration, or resumable execution;
- arbitrary caller-supplied GDT/IDT/page-table layouts or guest-controlled descriptor-table construction.

## Promotion rule

After the MMIO level interrupt lifecycle is integrated and exact merged-`main` CI is green, seal the fixed byte-register/GSI0 lifecycle. Do not farm more register values, ACK encodings, or duplicate fixed-GSI fixtures.

The next architecture audit should promote into a genuinely reusable frontier with a real second consumer. Highest-value candidates include a multi-device MMIO dispatch/registration layer that can host two independent executable devices, explicit programmable interrupt routing beyond fixed GSI0, or a timer/device source that exercises the same interrupt lifecycle without guest command polling. PCI/virtio, SMP, irqfd acceleration, migration, and broader machine models remain separate milestones and must earn implementation plus executable evidence.
