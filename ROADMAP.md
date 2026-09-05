# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO execution, long-mode virtual-MMIO composition, direct and controller-backed interrupt delivery, MMIO-device interrupt lifecycles, bounded multi-device MMIO registration/mapping, dual-source legacy-PIC routing, host-driven timer delivery through both direct `KVM_IRQ_LINE` and irqfd/eventfd, one ioeventfd-to-irqfd accelerated doorbell round trip, one synthetic guest-discovered PCI BAR-backed MMIO function, and one modern virtio-rng PCI split-ring request.

The modern virtio-rng request phase is integrated at commit `d35bb5e73b47ad1204793ed3119b56e2bbc9d605` through PR #90. Exact merged-main CI #435 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all fourteen earlier strict real-KVM gates, and the fifteenth strict virtio-rng PCI request gate. Its executable guest discovers modern virtio-rng identity and capabilities, negotiates `VIRTIO_F_VERSION_1`, configures one checked split queue, submits one writable descriptor, receives the deterministic eight-byte payload `RNGDATA!`, observes used tuple `1/0/8`, and completes with proof `PVNR`.

That one-request non-interrupting virtio transport phase is sealed. Do not farm more fixed payloads, descriptor counts, duplicate queues, or equivalent single-request variants merely to extend the phase number.

## Selected milestone — virtio-rng completion through bounded legacy INTx

The next architecture boundary is guest-visible request-completion interrupt ownership. Keep the integrated modern PCI transport, BAR, feature/status negotiation, queue layout, descriptor contract, deterministic payload and checked guest-memory processing unchanged, but make successful queue completion own virtio ISR bit 0 and one legacy INTx-style level interrupt through the already integrated in-kernel irqchip/PIC/LAPIC ExtINT path.

This is a deliberately bounded legacy-INTx completion proof. It is not an MSI/MSI-X, generic PCI interrupt-routing, irqfd-backed INTx, interrupt-suppression/event-index, packed-ring, interoperability, entropy-quality or full virtio-conformance claim.

Acceptance contract:

- preserve all fifteen integrated strict real-KVM gates and every existing long-mode, ELF64, MMIO, accelerated-event, PCI, virtio, diagnostic and Rust 1.74 MSRV contract;
- retain the integrated modern virtio-rng PCI identity, capabilities, BAR0 GPA `0x10000000`, `VIRTIO_F_VERSION_1`, one split queue, checked direct writable descriptor, deterministic `RNGDATA!` payload, used-ring update and notify-completion ordering;
- successful queue processing must set virtio ISR queue bit 0 only after payload and used-ring writes plus queue state commit; failed queue processing must not create a completion interrupt state;
- expose the ISR capability byte at BAR offset `0x200` with read-to-clear semantics: the first guest ISR read after successful completion must return `1`, and a later guest ISR read after handler return must return `0`;
- compose the existing `LongModeMmioInterruptLayout`, in-kernel irqchip, LAPIC ExtINT setup and bounded GSI0/vector `0x40` route; do not introduce a parallel interrupt-table or controller path;
- the deterministic guest initializes the legacy PIC, enables interrupts, performs the same modern virtio-rng discovery/negotiation/queue/request sequence as the integrated slice, and emits notify barrier `N` only after the serviceable notify MMIO write has architecturally completed;
- userspace must consume exactly one retained `VirtioQueueNotified { queue: 0 }` event at `N`, process exactly one queue completion successfully, and only then assert GSI0 level;
- the interrupt handler emits `I`, reads ISR at BAR+`0x200`, requires the returned value to equal `1`, and emits barrier `A` only after that serviceable ISR read has completed;
- userspace may deassert GSI0 only at `A`, never at the ISR `KVM_EXIT_MMIO` itself; deassert count must be exactly one and the line must not remain asserted at completion;
- after deassertion the handler issues master-PIC EOI and `iretq`; resumed main verifies used tuple `1/0/8` and payload `RNGDATA!`, reads ISR again and requires `0`, then emits `R` followed by final userspace completion barrier `D`;
- with an in-kernel local APIC, guest HLT is not a portable userspace terminal because KVM may retain the halted vCPU in-kernel until another wake event; therefore userspace must stop at `D` without another `KVM_RUN`. Observing `D` requires one re-entry after `R`, so `R` is committed without relying on serviceable-I/O RIP semantics;
- exact debug proof is `PVNIARD`; the two ISR reads extend the integrated nineteen virtio MMIO accesses to exactly twenty-one MMIO exits, while exact port-I/O count is nineteen (six PCI config cycles × two exits plus seven proof bytes);
- the exact execution budget remains forty completed exits: twenty-one MMIO plus nineteen port-I/O. The safety HLT byte after `D` is not executed and is not counted as an exit;
- exact host-visible completion state is descriptor id `0`, length `8`, `used.idx=1`, used id `0`, used len `8`, payload bytes `[82, 78, 71, 68, 65, 84, 65, 33]`, one assert, one deassert, GSI0, vector `0x40`, software-enabled LAPIC SPIV and unmasked ExtINT LINT0;
- completion-barrier RFLAGS must have architectural bit 1 and IF set;
- KVM-aware integration must independently validate completion fields, payload, proof bytes, exact twenty-one MMIO exits including both ISR reads, exact nineteen port-I/O exits, one assert/deassert lifecycle, LAPIC state and completion RFLAGS;
- stable CI must retain all fifteen integrated strict real-KVM gates unchanged and add an independent sixteenth virtio-rng completion-interrupt gate requiring GSI0/vector `0x40`, lifecycle `assert=1 deassert=1`, used tuple `1/0/8`, payload `RNGDATA!`, proof bytes `[80, 86, 78, 73, 65, 82, 68]`, nineteen port-I/O exits, twenty-one MMIO exits, semantic LAPIC ExtINT state and completion RFLAGS bit 1 plus IF;
- queue processing, ISR ownership/read-to-clear behavior, serviceable-MMIO completion ordering, GSI assertion/deassertion ownership, userspace completion-barrier ordering, guest-memory bounds, proof/state verification, exact exit accounting or MSRV failures remain hard and must not be swallowed, skipped into success, retried into success or hidden by changing expected values.

## Scope boundary

This milestone deliberately does **not** add:

- MSI, MSI-X, PCIe ECAM, arbitrary PCI interrupt routing, IOAPIC programming, shared INTx lines or irqfd-backed INTx acceleration;
- interrupt suppression, event-index, packed rings, indirect descriptors, descriptor chains, multiple requests, additional queues, periodic work or a general virtio scheduler;
- more virtio device types, a general guest-driver compatibility layer, BAR relocation/sizing, PCI bridges or hotplug;
- real entropy/randomness quality claims, cryptographic claims, full virtio conformance/interoperability claims, performance or latency claims;
- DMA/IOMMU infrastructure, SMP/multi-vCPU execution, migration, resumable execution or whole-VM snapshots.

## Promotion rule

After this single virtio-rng INTx completion lifecycle is integrated and exact merged-`main` CI is green, seal the fixed GSI0/vector0x40 legacy-INTx proof rather than farming repeated requests, more fixed ISR reads or duplicate interrupt vectors.

The next architecture audit should prefer a materially higher transport/control-plane frontier. Strong candidates include MSI/MSI-X only when PCI capability programming and executable interrupt delivery can be proven end to end, irqfd-backed device completion only if it changes ownership/acceleration semantics rather than duplicating the existing INTx proof, a second standards-shaped virtio device when it exercises a different queue/data contract, or PCIe/DMA/IOMMU/SMP work when one becomes a necessary executable prerequisite. Performance work remains separate and requires controlled benchmark evidence.
