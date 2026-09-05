# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO execution, long-mode virtual-MMIO composition, direct and controller-backed interrupt delivery, MMIO-device interrupt lifecycles, bounded multi-device MMIO registration/mapping, dual-source legacy-PIC routing, host-driven timer delivery through both direct `KVM_IRQ_LINE` and irqfd/eventfd, one ioeventfd-to-irqfd accelerated doorbell round trip, and one synthetic guest-discovered PCI BAR-backed MMIO function.

The synthetic PCI discovery phase is integrated at commit `98a3b3b61e3e674944f460ce66d7bce21b8fcc8f` through PR #89. Exact merged-main CI #412 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all thirteen earlier strict real-KVM gates, and the fourteenth strict PCI configuration/BAR gate. Its executable guest discovers synthetic function `00:01.0` through legacy PCI configuration mechanism #1, verifies vendor/device/class/BAR values, then writes the discovered BAR-backed GPA `0x10000000` and finishes with proof `PCBM`.

That synthetic one-function PCI phase is sealed. Do not farm more fixed BDFs, BAR values, or configuration registers merely to extend the phase number.

## Selected milestone — one modern virtio-rng PCI split request

The next architecture boundary is a truthful standards-shaped device transport. This milestone adds a separate modern virtio-rng PCI function rather than relabeling the sealed synthetic device. The guest must discover standard virtio PCI capabilities, negotiate the bounded feature/status contract, configure one split request queue in checked guest RAM, notify queue 0, and observe one deterministic device completion end to end.

This remains a deliberately bounded first virtio slice. The fixed payload is deterministic test evidence only; it is not an entropy-quality, cryptographic, interoperability, or full virtio-conformance claim.

Acceptance contract:

- preserve all fourteen integrated strict real-KVM gates and every existing long-mode, ELF64, MMIO, interrupt, accelerated-event, synthetic-PCI, diagnostic, and Rust 1.74 MSRV contract;
- keep the existing synthetic `vendor=0xcafe/device=0x0001` function unchanged as an independent historical proof;
- expose a separate modern virtio-rng PCI function with vendor `0x1af4`, device `0x1044` (virtio device type 4), revision 1, and a PCI capabilities list;
- expose standard vendor-specific virtio PCI capabilities for common configuration, queue notification, and ISR status, all backed by one bounded memory BAR at GPA `0x10000000`;
- offer only `VIRTIO_F_VERSION_1` and require the bounded ACKNOWLEDGE -> DRIVER -> FEATURES_OK -> DRIVER_OK progression before queue notification;
- expose exactly one split virtqueue (`requestq`, queue 0) with bounded power-of-two size and explicit descriptor/driver/device GPAs;
- require one direct descriptor marked device-writable; NEXT, INDIRECT, read-only descriptors, out-of-range descriptor indices, too-small buffers, address overflow, and malformed register accesses remain hard failures;
- consume queue state through checked `GuestMemory` reads/writes, fill exactly the fixed eight-byte payload `RNGDATA!`, write one used element with descriptor id 0 and length 8, and advance `used.idx` to 1;
- preserve the repository completion invariant for serviceable notify MMIO: queue processing occurs only after the subsequent explicit guest debug-port `N` barrier proves the notify write completed;
- the deterministic guest must discover PCI identity/capability pointers/BAR through six CF8/CFC cycles, emit `P`, negotiate the virtio common configuration and emit `V`, build the one-entry split ring, notify queue 0, emit completion barrier `N`, verify used-ring fields and payload inside guest code, emit `R`, and halt;
- exact success proof is `PVNR`; exact host-visible state is driver features `0x100000000`, device status `0x0f`, queue enabled, `used.idx=1`, used id 0, used len 8, payload bytes `[82, 78, 71, 68, 65, 84, 65, 33]`, sixteen port-I/O exits, and nineteen MMIO exits;
- the execution budget is exact: six PCI config cycles × two exits + nineteen MMIO exits + four proof outputs + one HLT = 36; serviceable MMIO completion requires re-entry but does not invent an extra exit;
- KVM-aware integration must independently validate PCI cycles, proof bytes, final status/features/queue state, used-ring fields, payload, notify MMIO metadata, HLT terminal state, and architectural RFLAGS bit 1;
- stable CI must retain all fourteen integrated strict real-KVM gates unchanged and add an independent fifteenth virtio-rng gate requiring modern identity `0x1af4/0x1044`, BAR0 `0x10000000`, features `0x100000000`, status `0x0f`, queue enabled, used tuple `1/0/8`, payload `RNGDATA!`, proof bytes `[80, 86, 78, 82]`, sixteen port-I/O exits, nineteen MMIO exits, HLT, and architectural RFLAGS bit 1;
- queue negotiation, descriptor safety, guest-memory bounds, notify completion ordering, proof/state verification, exact exit accounting, or MSRV failures remain hard and must not be swallowed, skipped into success, retried into success, or hidden by changing expected values.

## Scope boundary

This milestone deliberately does **not** add:

- packed rings, indirect descriptors, descriptor chains, multiple queues, notification-data, ring reset, admin queues, or arbitrary guest-driver compatibility;
- INTx queue completion interrupts, MSI, MSI-X, PCIe ECAM, BAR relocation/sizing, multiple BARs, bridges, hotplug, or a general PCI bus enumerator;
- device-specific configuration beyond the bounded virtio-rng identity, DMA/IOMMU infrastructure, or a general DMA engine;
- entropy/randomness quality claims, cryptographic claims, performance/latency claims, or full virtio conformance/interoperability claims;
- SMP/multi-vCPU execution, migration, resumable execution, or whole-VM snapshots.

## Promotion rule

After this one-request virtio-rng phase is integrated and exact merged-`main` CI is green, seal the single-request transport rather than farming descriptor counts, fixed payloads, or duplicate queues.

The next architecture audit should prefer a cross-layer virtio capability that materially changes execution. A strong candidate is virtio request-completion interrupt delivery, combining the standards-shaped queue with the already-integrated controller/irqfd stack, only if the guest-visible interrupt programming and acknowledgement semantics can be proven end to end. MSI/MSI-X, a second standard virtio device, PCIe ECAM, DMA/IOMMU, SMP, migration, and performance work remain separate frontiers unless one becomes a necessary executable prerequisite.
