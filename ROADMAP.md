# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO device execution, bounded long-mode virtual-MMIO composition, bounded direct long-mode interrupt delivery, one controller-backed GSI0 route through KVM's in-kernel x86 irqchip, one MMIO-device-generated interrupt path, one stateful device-owned MMIO level-interrupt lifecycle, and bounded two-device MMIO registration/dispatch with two independent virtual-MMIO page mappings.

The multi-device phase is integrated at commit `37d814648c06f11b8c182cdee8f4e2d541156bb4` through PR #84. Exact merged-main CI #377 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all eight earlier strict real-KVM gates, and the ninth strict multi-device MMIO gate. Its executable proof hosts VA `0x500000`→GPA `0x10000000` and VA `0x501000`→GPA `0x10001000` in one VM, records independent writes `[X]` and `[Y]`, observes five exact MMIO exits, returns proof `ABAM`, and halts at RIP `0x1002b`.

That two-device registry/mapping phase is sealed. Do not farm a third identical byte device, more fixed address variants, or duplicate mapping tests merely to extend the phase number.

## Selected milestone — dual-source MMIO level interrupts through distinct legacy-PIC routes

The next architecture boundary is interrupt-source identity and routing. Existing level-interrupt execution owns one MMIO source, one GSI0 line, and one vector `0x40`; the integrated multi-device bus now provides the missing second independent source. This milestone promotes the interrupt table, MMIO event surface, and userspace routing policy together, then proves both sources through the existing in-kernel x86 irqchip on real KVM.

This is a deliberately bounded legacy-PIC routing model, not a claim of arbitrary `KVM_SET_GSI_ROUTING`. The deterministic guest programs the master PIC to vectors `0x40..0x47`, unmasks only IRQ0 and IRQ1, and userspace assigns the two MMIO sources to GSI0 and GSI1 respectively.

Acceptance contract:

- preserve all nine integrated strict real-KVM gates and every existing long-mode, ELF64, MMIO, interrupt, snapshot, CPU-policy, diagnostic, and MSRV contract;
- promote `LongModeInterruptLayout` from one installed IDT gate to a validated bounded gate collection while retaining the existing single-gate constructor as a compatibility wrapper;
- reject an empty gate set, exception-reserved vectors, duplicate vectors, handler addresses outside the identity map, and handler collisions with bootstrap/GDT/IDT tables before installation;
- set the IDT limit from the highest installed vector and write every validated gate into the same existing IDT page;
- retain MMIO device identity when consuming device events: the routing layer must receive both the registered device base and the event kind rather than infer a source from event order;
- allow multiple level-interrupt byte devices to be registered through the existing overlap-checked MMIO registry; COMMAND/STATUS/ACK extents remain three bytes and overlapping registrations remain hard failures;
- define a bounded legacy master-PIC route set that accepts only GSI `0..7`, derives vector `0x40 + gsi`, and rejects empty, duplicate-source, duplicate-GSI, and out-of-range route sets;
- deterministic source 0 uses virtual page `0x500000`, GPA `0x10000000`, GSI0, vector `0x40`, and handler `0x11000`;
- deterministic source 1 uses virtual page `0x501000`, GPA `0x10001000`, GSI1, vector `0x41`, and handler `0x12000`;
- one guest must execute two complete level lifecycles sequentially in the same VM: source0 COMMAND→assert→handler0→STATUS=1→ACK→deassert→EOI→IRETQ, then source1 COMMAND→assert→handler1→STATUS=1→ACK→deassert→EOI→IRETQ;
- the host may assert/deassert a GSI only after the same explicit post-MMIO completion barriers used by the integrated single-source lifecycle; serviceable `KVM_EXIT_MMIO` itself is not treated as a portable architectural commit point;
- every assert and deassert must be resolved from a source-tagged MMIO event through the route set, and deassert must resolve to the same route that owned the preceding assert;
- exact MMIO metadata is six one-byte exits: source0 COMMAND write, source0 STATUS read, source0 ACK write, source1 COMMAND write, source1 STATUS read, source1 ACK write;
- exact host-visible writes are `[W, 1]` for both sources; exact event counts are two asserts and two deasserts;
- exact debug-port proof is `A0SCMB1TEND`: `0` and `1` are emitted by different IDT handlers and therefore distinguish vector `0x40` from vector `0x41`; `S`/`T` prove each handler consumed STATUS=1; `C`/`E` are ACK completion barriers; `M`/`N` prove return to the interrupted main path; `D` is the final userspace synchronization barrier;
- LAPIC SPIV must remain software-enabled and LINT0 must remain unmasked ExtINT; both armed observations and final completion must have architectural RFLAGS bit 1 and IF set;
- KVM-aware integration must independently validate both route tuples, all six MMIO exits, both write traces, all eleven byte-wide debug-port exits, event counts, LAPIC state, and RFLAGS;
- stable CI must retain the nine integrated strict real-KVM gates and add an independent tenth dual-source gate requiring route `0x10000000→GSI0→0x40`, route `0x10001000→GSI1→0x41`, two asserts, two deasserts, six MMIO exits, both writes `[87, 1]`, proof `[65, 48, 83, 67, 77, 66, 49, 84, 69, 78, 68]`, semantic LAPIC ExtINT state, and IF at both armed points and completion;
- routing, source identity, gate installation, MMIO completion ordering, STATUS/ACK semantics, GSI ownership, proof, or architectural-state failures remain hard failures and must not be swallowed, skipped, retried into success, or hidden by changed test expectations.

## Scope boundary

This milestone deliberately does **not** add:

- arbitrary `KVM_SET_GSI_ROUTING`, IOAPIC programming, MSI/MSI-X, x2APIC, or a general interrupt-routing API;
- more than the bounded master-PIC GSI `0..7` model, slave-PIC routing, shared GSIs, level-sharing semantics, priority policy, or interrupt scheduling;
- asynchronous timers, eventfd/irqfd/ioeventfd acceleration, or host-thread device workers;
- PCI/PCIe configuration space, BAR enumeration, virtio transport, DMA, or IOMMU;
- a trait-object plugin framework, dynamic hotplug, bus discovery protocol, or arbitrary device ABI;
- SMP, cross-vCPU delivery, multiple vCPUs, migration, resumable execution, or whole-VM snapshots;
- an unbounded guest virtual-address allocator or arbitrary caller-supplied page-table hierarchy.

## Promotion rule

After the dual-source route is integrated and exact merged-`main` CI is green, seal the two-fixed-source legacy-PIC proof rather than adding GSI2/GSI3 clones.

The next architecture audit should choose a genuinely different interaction pattern. Strong candidates are an asynchronous timer/device source that reuses the established routing and level lifecycle, a minimal PCI/virtio transport that supplies a real device-discovery/configuration surface, or a more general interrupt-controller/routing phase only when it can be backed by executable KVM evidence. SMP, irqfd acceleration, DMA/IOMMU, and migration remain separate frontiers.
