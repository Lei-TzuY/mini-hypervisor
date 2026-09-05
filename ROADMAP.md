# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO execution, long-mode virtual-MMIO composition, direct and controller-backed interrupt delivery, MMIO-device interrupt lifecycles, bounded multi-device MMIO registration/mapping, dual-source legacy-PIC routing, host-driven timer delivery through both direct `KVM_IRQ_LINE` and irqfd/eventfd, one ioeventfd-to-irqfd accelerated doorbell round trip, one synthetic guest-discovered PCI BAR-backed MMIO function, one modern virtio-rng PCI split-ring request, and one virtio-rng completion lifecycle through bounded legacy INTx.

The legacy-INTx virtio-rng completion phase is integrated at commit `2248b3d01ce18d57b9d5bebf6fa2d75764d7c058` through PR #91. Exact merged-main CI #447 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all fifteen earlier strict real-KVM gates, and the sixteenth strict virtio-rng completion-interrupt gate. Its executable path preserves the modern virtio-rng queue/data contract, fills `RNGDATA!`, commits used tuple `1/0/8`, owns ISR queue bit 0, delivers one GSI0/vector `0x40` level interrupt, reads ISR `1` in the handler and `0` after return, and completes with proof `PVNIARD`.

That fixed legacy-INTx completion phase is sealed. Do not farm repeated requests, duplicate fixed ISR reads, extra fixed GSI/vector variants, or equivalent INTx-only completion paths merely to extend the phase number.

## Selected milestone — virtio-rng completion through guest-programmed PCI MSI

The next architecture boundary is a materially different PCI interrupt transport. Keep the integrated modern virtio-rng queue/data contract, checked guest-memory processing, deterministic payload, used-ring update and ISR read-to-clear ownership, but replace the fixed legacy INTx delivery step with one guest-programmed 32-bit PCI MSI message delivered through `KVM_SIGNAL_MSI`.

This is deliberately one bounded, single-message 32-bit MSI proof. It is not an MSI-X, multiple-message MSI, arbitrary PCI routing, APIC-priority, repeated-interrupt, interoperability, full virtio-conformance, performance, SMP, DMA/IOMMU or migration claim.

Acceptance contract:

- preserve all sixteen integrated strict real-KVM gates and every existing long-mode, ELF64, MMIO, accelerated-event, PCI, virtio, diagnostic and Rust 1.74 MSRV contract;
- expose one MSI capability after the existing virtio PCI capability chain, with capability id `0x05`, a 32-bit message address field, one 16-bit message-data field, and only the single MSI-enable control bit;
- the legacy non-MSI virtio-rng constructor must retain its terminated capability chain and must not accidentally expose MSI state;
- PCI Mechanism #1 must service dword writes only for the bounded MSI control/address/data fields on the MSI-enabled virtio-rng function; synthetic functions, read-only fields, unknown offsets, unsupported control bits and malformed data writes remain hard failures;
- guest code, not host fixture constants, must program MSI address `0xfee00000`, message data/vector `0x50`, and enable state through real PCI config cycles; userspace delivery must read back and consume that exact guest-programmed message;
- require `KVM_CAP_SIGNAL_MSI` as a hard runtime capability and use the exact 32-byte Linux `struct kvm_msi` ABI; missing capability, ioctl failure, zero/coalesced delivery, or invalid return counts remain hard failures;
- retain the integrated modern virtio-rng identity, BAR0 GPA `0x10000000`, `VIRTIO_F_VERSION_1`, one checked split queue, direct writable descriptor, deterministic `RNGDATA!` payload, queue notification ownership, used-ring update, status/feature verification and ISR queue-bit ownership;
- the deterministic guest must discover the PCI capability chain and BAR, program and enable MSI, emit proof byte `P`, complete the same virtio setup and request path, then emit notify barrier `N` only after the serviceable notify MMIO write has architecturally completed;
- userspace must consume exactly one `VirtioQueueNotified { queue: 0 }` event at `N`, process exactly one queue completion successfully, then retrieve the enabled MSI message from the PCI config model and call `KVM_SIGNAL_MSI` exactly once with that guest-programmed address/data;
- MSI delivery uses vector `0x50` and a distinct long-mode IDT handler at GPA `0x12000`; the MSI handler emits `M`, reads the virtio ISR capability byte and requires queue bit value `1`, then emits barrier `A` and returns with `iretq` without issuing legacy PIC EOI;
- resumed main must verify used tuple `1/0/8`, payload `RNGDATA!`, read ISR again and require `0`, emit `R`, then final userspace completion barrier `D`;
- exact debug proof is `PVNMARD`; exact userspace accounting is twenty-seven port-I/O exits (seven PCI reads, three PCI writes, seven proof bytes) and twenty-one MMIO exits, including both ISR reads;
- exact host-visible completion state is descriptor id `0`, length `8`, used tuple `1/0/8`, payload bytes `[82, 78, 71, 68, 65, 84, 65, 33]`, MSI address `0xfee00000`, MSI data/vector `0x50`, and exactly one successful MSI delivery;
- software-enabled LAPIC SPIV must remain observable; completion-barrier RFLAGS must contain architectural bit 1 and IF. Do not require legacy LINT0 ExtINT semantics for MSI delivery because the MSI path is not the legacy PIC/ExtINT transport;
- KVM-aware integration must independently validate the guest-programmed message, one MSI delivery, queue completion fields, payload, proof, exact twenty-seven port-I/O exits, exact twenty-one MMIO exits including ISR read-to-clear, LAPIC SPIV and completion RFLAGS;
- stable CI must retain all sixteen integrated strict real-KVM gates unchanged and add an independent seventeenth virtio-rng MSI gate requiring address `0xfee00000`, data/vector `0x50`, delivery count `1`, used tuple `1/0/8`, payload `RNGDATA!`, proof bytes `[80, 86, 78, 77, 65, 82, 68]`, twenty-seven port-I/O exits, twenty-one MMIO exits, software-enabled LAPIC SPIV and completion RFLAGS bit 1 plus IF;
- PCI configuration ownership, MSI enable gating, capability validation, queue processing, ISR ownership/read-to-clear behavior, serviceable-MMIO completion ordering, `KVM_SIGNAL_MSI` delivery, proof/state verification, exact exit accounting or MSRV failures remain hard and must not be swallowed, skipped into success, retried into success or hidden by changing expected values.

## Scope boundary

This milestone deliberately does **not** add:

- MSI-X, multiple-message MSI, 64-bit MSI messages, per-vector masking, arbitrary `KVM_SET_GSI_ROUTING`, PCIe ECAM, IOAPIC programming or a generic PCI interrupt-routing framework;
- repeated MSI delivery, interrupt-priority/arbitration claims, shared interrupt semantics, irqfd-backed MSI or eventfd acceleration for this path;
- interrupt suppression, event-index, packed rings, indirect descriptors, descriptor chains, multiple requests, additional queues, periodic work or a general virtio scheduler;
- more virtio device types, a general guest-driver compatibility layer, BAR relocation/sizing, PCI bridges or hotplug;
- entropy/randomness quality, cryptographic, full virtio conformance/interoperability, performance or latency claims;
- DMA/IOMMU infrastructure, SMP/multi-vCPU execution, migration, resumable execution or whole-VM snapshots.

## Promotion rule

After this single guest-programmed virtio-rng MSI completion path is integrated and exact merged-`main` CI is green, seal the fixed one-vector MSI proof rather than farming more fixed MSI addresses, vectors or repeated one-shot completions.

The next architecture audit should prefer a materially higher transport or device-model frontier. Strong candidates include MSI-X only when a guest-programmed table/PBA and executable vector delivery can be proven end to end, a second standards-shaped virtio device that exercises a different queue/data contract, or PCIe/DMA/IOMMU/SMP work when one becomes a necessary executable prerequisite. Performance work remains separate and requires controlled benchmark evidence.
