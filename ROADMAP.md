# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO execution, long-mode virtual-MMIO composition, direct long-mode interrupt delivery, one in-kernel x86 irqchip/GSI route, MMIO-device interrupt delivery, stateful device-owned level-interrupt lifecycle, bounded two-device MMIO registration/mapping, two independently routed MMIO level-interrupt sources, one host-driven asynchronous timer wakeup, one KVM irqfd/eventfd accelerated timer transport, and one KVM ioeventfd-to-irqfd accelerated doorbell round trip.

The accelerated doorbell phase is integrated at commit `f0a7e1f9786ae39cfef4c2709b63c09d0c65863d` through PR #88. Exact merged-main CI #401 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all twelve earlier strict real-KVM gates, and the thirteenth strict ioeventfd-to-irqfd round-trip gate. Its executable proof consumes the exact MMIO doorbell GPA `0x10000000` / value `0x5a` inside KVM through `KVM_IOEVENTFD`, bridges exactly one event to an irqfd eventfd, and reaches the existing GSI0/vector `0x40` handler with proof `RATWD` without a userspace MMIO exit.

That fixed accelerated doorbell bridge is sealed. Do not farm more eventfd flags, fixed addresses, queue clones, or delay variants merely to extend the phase number.

## Selected milestone — bounded legacy PCI configuration discovery and BAR-backed MMIO execution

The next architecture boundary is guest-visible device discovery/configuration. The integrated MMIO/event transports expose fixed addresses only to the host fixture; this milestone adds one deliberately synthetic PCI function that the guest can discover through x86 PCI configuration mechanism #1 and whose BAR0 describes the already-established unbacked MMIO GPA.

This is a bounded synthetic PCI model, not a virtio, PCIe, or PCI conformance claim. The guest-visible identity is intentionally nonstandard (`vendor=0xcafe`, `device=0x0001`, vendor-specific class `0xff`) so the project cannot accidentally imply interoperability with a real device specification.

Acceptance contract:

- preserve all thirteen integrated strict real-KVM gates and every existing long-mode, ELF64, MMIO, interrupt, snapshot, CPU-policy, accelerated-event, diagnostic, and Rust 1.74 MSRV contract;
- implement only legacy x86 PCI configuration mechanism #1 on ports CF8/CFC, and only 32-bit, count-one configuration cycles in this slice;
- expose exactly one synthetic function at `00:01.0`; disabled config selection or an absent BDF must read `0xffff_ffff`;
- expose read-only vendor/device identity, class/revision, and BAR0 configuration; unsupported config-data writes remain hard failures rather than silently mutating state;
- BAR0 is a 32-bit non-prefetchable memory BAR whose address bits resolve to GPA `0x10000000`, reusing the existing bounded unbacked MMIO physical page;
- preserve the existing debug port while routing CF8/CFC through the same `PortIoBus`; unknown or malformed accesses retain existing typed `PortIoError` behavior;
- the deterministic long-mode guest must select/read identity (`0x80000800`), class/revision (`0x80000808`), and BAR0 (`0x80000810`) through real OUT/IN cycles, compare every result inside guest code, and branch to explicit proof byte `F` on any mismatch;
- only after those three guest checks succeed may it emit `P`, `C`, `B`, write byte `W` through virtual address `0x500000` to the discovered BAR0-backed GPA, emit `M`, and halt;
- exact success proof is `PCBM`; exact BAR write trace is `[W]`; exact execution has ten port-I/O exits, one MMIO write exit at GPA `0x10000000`, and HLT at RIP `0x10065` with architectural RFLAGS bit 1 set;
- KVM-aware integration must independently validate all three CF8/CFC selector/data cycles, all four proof bytes, exact MMIO metadata, write trace, and terminal state; KVM unavailable/permission may skip only under the repository's ordinary environment-sensitive integration convention, while every other error remains hard;
- stable CI must retain all thirteen integrated strict real-KVM gates unchanged and add an independent fourteenth PCI discovery gate requiring function `00:01.0`, vendor/device `0xcafe/0x0001`, class `0xff`, BAR0 `0x10000000`, writes `[87]`, proof bytes `[80, 67, 66, 77]`, ten port-I/O exits, one MMIO exit, and HLT RIP `0x10065` with architectural RFLAGS bit 1 set;
- selector encoding, PCI identity/class/BAR values, port-I/O metadata, guest compare/failure branches, MMIO translation, proof, write trace, terminal state, or MSRV failures remain hard and must not be swallowed, retried into success, or hidden by changing expected values.

## Scope boundary

This milestone deliberately does **not** add:

- PCI command/status writes, BAR sizing probes, BAR relocation, multiple BARs, multifunction devices, bus enumeration, bridges, subordinate buses, or hotplug;
- PCIe ECAM, ACPI/MCFG, PCI Express capabilities, power management, or configuration-space capability lists;
- real virtio vendor/device IDs, virtio PCI common/device/notify/ISR configuration, feature negotiation, queue setup, descriptors, DMA, IOMMU, or any virtio conformance/interoperability claim;
- MSI/MSI-X, arbitrary KVM GSI routing, IOAPIC programming, x2APIC, irqfd resample, or shared interrupt lines;
- periodic timers, scheduler framework, SMP/multi-vCPU delivery, migration, resumable execution, or performance/latency claims.

## Promotion rule

After the PCI discovery/BAR execution proof is integrated and exact merged-`main` CI is green, seal this one-function mechanism-#1 phase rather than farming more fixed config registers, BDFs, or BAR values.

The next architecture audit should prefer a materially higher device-model frontier. A strong candidate is a minimal, truthful virtio PCI transport only if it can implement real standard identity/capability/feature/queue semantics with executable evidence rather than relabeling the synthetic device. Another valid frontier is a PCI interrupt transport (for example MSI/MSI-X) only when its controller/routing prerequisites and guest-visible programming model can be proven end to end. BAR relocation/sizing, PCIe ECAM, SMP, DMA/IOMMU, migration, and performance work remain separate phases unless one becomes a necessary executable prerequisite.
