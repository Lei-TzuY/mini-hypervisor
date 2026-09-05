# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO device execution, bounded long-mode virtual-MMIO composition, bounded direct long-mode interrupt delivery, one bounded controller-backed GSI0 route through KVM's in-kernel x86 irqchip, one bounded MMIO-device-generated interrupt path, and one stateful device-owned MMIO level-interrupt lifecycle.

The integrated level-interrupt path exposes fixed byte-wide COMMAND/STATUS/ACK registers, owns one assert/deassert lifecycle, preserves KVM MMIO completion barriers before changing GSI level, enters vector `0x40` through the established PIC/LAPIC route, proves handler-side STATUS observation and ACK, then resumes guest execution. Exact merged-main CI #370 on commit `67923feeffc7a813c99e38966db8d7e216fc4e67` retains all earlier proofs and adds the level lifecycle proof `AISCMD` with one assert event, one deassert event, writes `[W, 1]`, semantic LAPIC ExtINT state, and IF set at the armed/completion observations.

Merged-main CI therefore requires eight strict real-KVM executable gates through the device-owned level-interrupt phase. That phase is sealed: do not farm more ACK encodings, fixed-GSI variants, or duplicate lifecycle fixtures.

## Selected milestone — multi-device MMIO dispatch and virtual mapping

The next architecture boundary is no longer interrupt semantics. The current MMIO bus and long-mode virtual-MMIO layout historically encoded one device at a time. This milestone promotes those paths into a bounded reusable composition that hosts two independent executable MMIO devices in one VM without duplicating the common run loop or page-table machinery.

Acceptance contract:

- preserve every existing long-mode, ELF64, MMIO, virtual-MMIO, direct-interrupt, irqchip/GSI, one-shot device interrupt, level-interrupt, CPU-policy, snapshot, diagnostic, and strict real-KVM contract;
- evolve `MmioBus` from one optional byte device into a bounded registered-device collection while keeping existing single-device constructors source-compatible;
- registration must reject guest-physical address-range overflow and overlap before mutating bus state; adjacent non-overlapping ranges remain valid;
- dispatch must select the device whose registered half-open range contains the MMIO exit address and preserve each device's independent read value, write trace, and event state;
- multi-device observation must be explicit: a caller may query writes by registered base address; the legacy unqualified `writes()` accessor must not silently choose one device when more than one is registered;
- preserve the three-register COMMAND/STATUS/ACK extent of the level-interrupt device when checking overlaps, so another registered byte device cannot occupy STATUS or ACK;
- promote the long-mode MMIO boot layout from one fixed virtual-page/device-GPA pair to a validated list of MMIO page mappings while retaining the original single-device constructor as a one-mapping compatibility wrapper;
- each MMIO virtual page must be 4 KiB aligned and lie inside the existing bounded alias window; each device GPA must be 4 KiB aligned, must not overflow, and must remain outside registered RAM;
- duplicate MMIO virtual pages are hard configuration failures; page-table installation must reuse the existing alias PT and install one PTE per validated mapping rather than create another paging flow;
- deterministic executable proof uses two mappings in one VM: virtual `0x500000` to GPA `0x10000000`, and virtual `0x501000` to GPA `0x10001000`;
- the first device returns byte `A` and records write `X`; the second returns byte `B` and records write `Y`;
- guest execution must interleave the two devices rather than access them in isolated phases: first-device write `X`, first-device read, second-device write `Y`, second-device read, first-device read again, then completion byte `M` and HLT;
- exact MMIO metadata is therefore five one-byte exits in order: first GPA write `X`, first GPA read, second GPA write `Y`, second GPA read, first GPA read;
- exact host-visible device state is first writes `[X]` and second writes `[Y]`; exact debug proof is `ABAM`, where the repeated final `A` proves dispatch returned to the first device after servicing the second;
- terminal evidence is `KVM_EXIT_HLT` at RIP `0x10029` with architectural RFLAGS bit 1 set;
- KVM-aware integration must independently validate both virtual/GPA constants, all five MMIO metadata records, both write traces, all four byte-wide debug-port proof exits, HLT RIP, and RFLAGS;
- stable CI must retain all eight integrated strict real-KVM gates and add an independent ninth strict multi-device MMIO gate that requires first writes `[88]`, second writes `[89]`, five MMIO exits, proof `[65, 66, 65, 77]`, and HLT RIP `0x10029`;
- registration, mapping, dispatch, MMIO response, proof, terminal-state, or architectural-state failures remain hard failures and must not be retried, swallowed, skipped, or converted to best-effort success.

## Scope boundary

This milestone deliberately does **not** add:

- a trait-object device framework, arbitrary plugin ABI, dynamic hotplug, bus enumeration, or device discovery protocol;
- arbitrary-length BAR regions, PCI configuration space, PCI/PCIe topology, MSI/MSI-X, virtio transport, DMA, or IOMMU;
- a general event queue, interrupt scheduler, programmable GSI routing table, additional interrupt controller model, or a second independent interrupt source;
- irqfd/ioeventfd/eventfd acceleration;
- multiple RAM slots, memory hotplug, whole-VM snapshots, migration, or resumable execution;
- SMP, x2APIC, cross-vCPU routing, or multiple vCPUs;
- arbitrary caller-supplied page-table hierarchy construction or an unbounded guest virtual address allocator.

## Promotion rule

After multi-device MMIO composition is integrated and exact merged-`main` CI is green, seal the two-device registry/mapping proof rather than farming more fixed addresses or a third identical byte device.

The next architecture audit should select a frontier that introduces a genuinely new interaction pattern. Strong candidates are programmable interrupt routing with a real second source, a timer/device source that exercises the established level lifecycle asynchronously, or a minimal PCI/virtio transport only if it can be delivered as an executable cross-layer slice. SMP, irqfd acceleration, migration, and broad machine-model work remain separate milestones and must earn implementation plus executable evidence.
