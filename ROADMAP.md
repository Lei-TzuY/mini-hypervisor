# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO execution, long-mode virtual-MMIO composition, direct and controller-backed interrupt delivery, MMIO-device interrupt lifecycles, bounded multi-device MMIO registration/mapping, dual-source legacy-PIC routing, host-driven timer delivery through direct `KVM_IRQ_LINE` and irqfd/eventfd, one ioeventfd-to-irqfd accelerated doorbell round trip, guest-discovered PCI BAR-backed MMIO, one modern virtio-rng PCI split-ring request, virtio-rng completion through bounded legacy INTx, and one guest-programmed PCI MSI completion path.

The virtio-rng MSI completion phase is integrated at commit `e5bb73f314b05bbb9471ed7a8a90471c185f0281` through PR #92. Exact merged-main CI #462 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all sixteen earlier strict real-KVM gates, and the seventeenth strict virtio-rng MSI-completion gate. That path preserves the checked virtio-rng queue/data contract while replacing fixed INTx delivery with one guest-programmed 32-bit MSI message delivered through `KVM_SIGNAL_MSI`.

That one-vector virtio-rng MSI phase is sealed. Do not farm more fixed MSI addresses/vectors, duplicate one-shot completions, or repeated rng requests merely to extend the phase number.

## Selected milestone — bounded virtio-blk sector read

The next architecture boundary is a second standards-shaped virtio device with a materially different queue/data contract. This milestone adds one bounded modern virtio-blk PCI function and proves one `VIRTIO_BLK_T_IN` request for sector 0 through a checked three-descriptor split-ring chain.

This is deliberately a first read-only storage slice. It is not a write/durability, repeated-I/O, interrupt/MSI, multi-sector, packed-ring, indirect-descriptor, full virtio-blk conformance, performance, PCIe, DMA/IOMMU, SMP or migration claim.

Acceptance contract:

- preserve all seventeen integrated strict real-KVM gates and every existing long-mode, ELF64, MMIO, accelerated-event, PCI, virtio-rng, interrupt, diagnostic and Rust 1.74 MSRV contract;
- expose modern virtio-blk PCI identity with vendor `0x1af4`, device `0x1042`, one BAR0, the bounded common/notify/ISR capability chain, and a device-specific configuration capability exposing capacity `1` sector;
- negotiate only `VIRTIO_F_VERSION_1`, expose exactly queue 0, and preserve the bounded virtio status/feature/queue state-machine invariants used by the integrated transport;
- support exactly one first-slice `VIRTIO_BLK_T_IN` request for sector 0 through three descriptors: a readable 16-byte request header, a writable 512-byte data buffer, and a writable one-byte status buffer;
- require exact descriptor NEXT/WRITE direction flags, distinct in-range descriptor indices, minimum descriptor lengths, request type `IN`, reserved field `0`, and sector less than the advertised capacity before accepting the request;
- use one deterministic read-only 512-byte backing sector whose first 16 bytes are `BLK-SECTOR-0000!` and last eight bytes are `BLKEND!!`;
- successful processing must copy the entire 512-byte sector to the writable data descriptor, write `VIRTIO_BLK_S_OK` to the status descriptor, publish used element `{ id=head, len=513 }`, increment `used.idx`, clear the pending notify ownership, and assert the virtio ISR queue bit;
- guest code must discover PCI identity/capabilities/BAR through real Mechanism #1 cycles, read capacity from the device-specific MMIO configuration, negotiate features/status, configure descriptor/avail/used addresses, enable queue 0, materialize the three-descriptor request in guest RAM, notify queue 0, then validate used/data/status and ISR state after host processing;
- the serviceable notify MMIO exit is not itself treated as completed architecture: guest emits an explicit `N` debug-port barrier after re-entry; userspace may consume the pending `VirtioQueueNotified { queue: 0 }` event and process guest memory only at that barrier;
- exact host-visible completion state is descriptor id `0`, length `513`, sector `0`, used tuple `1/0/513`, request status `0`, negotiated features `VIRTIO_F_VERSION_1`, queue enabled, and data exactly equal to the deterministic backing sector;
- exact debug-port proof is `PBNR`: `P` proves PCI capability/BAR discovery, `B` proves transport negotiation/queue readiness, `N` is the post-notify completion barrier, and `R` proves the guest observed the used ring, status/data signatures and ISR queue bit;
- exact port-I/O accounting is eighteen exits: seven PCI configuration read cycles (fourteen exits) plus four one-byte debug outputs; exact MMIO accounting is twenty-one exits including capacity read, common/queue programming, notify and ISR read-to-clear;
- execution must terminate at the dynamically computed HLT RIP with architectural RFLAGS bit 1 set; unexpected exits, missing queue events, duplicate processing, stale device events or changed exit accounting remain hard failures;
- KVM-aware integration must independently validate completion id/length/sector, negotiated features, queue enabled state, used tuple, request status, all 512 data bytes, proof `PBNR`, exact port-I/O/MMIO counts, terminal HLT and architectural RFLAGS bit 1;
- stable CI must retain all seventeen integrated strict real-KVM gates unchanged and add an independent eighteenth virtio-blk gate requiring PCI identity `0x1af4/0x1042`, capacity `1`, features `0x100000000`, queue enabled, completion `0/513/0`, used tuple `1/0/513`, request status `0`, deterministic data-boundary signatures, proof bytes `[80, 66, 78, 82]`, eighteen port-I/O exits, twenty-one MMIO exits and terminal HLT/RFLAGS bit 1;
- PCI ownership, capacity/config semantics, queue negotiation, descriptor validation, guest-memory processing, used-ring publication, ISR ownership/read-to-clear, notify completion ordering, exact proof/accounting or MSRV failures must not be swallowed, skipped into success, retried into success or hidden by changing expected values.

## Scope boundary

This milestone deliberately does **not** add:

- `VIRTIO_BLK_T_OUT`, flush, discard, write-zeroes, barriers, persistence, filesystem semantics or any durability claim;
- more than one sector, repeated requests, multiple queues, queue wraparound, indirect descriptors, event-index, packed rings or interrupt suppression;
- a virtio-blk completion interrupt, INTx, MSI/MSI-X, irqfd acceleration, arbitrary PCI routing or additional interrupt-controller behavior;
- arbitrary guest-driver compatibility, full virtio-blk conformance/interoperability, hotplug, PCI bridges, PCIe ECAM or BAR relocation/sizing;
- controlled storage benchmarks, throughput/latency claims, caching/writeback policy or host-file/block-device backends;
- DMA/IOMMU infrastructure, SMP/multi-vCPU execution, migration, resumable execution or whole-VM snapshots.

## Promotion rule

After this bounded sector-read path is integrated and exact merged-`main` CI is green, seal the one-request read slice instead of farming more fixed sectors or duplicate descriptor-layout tests.

The next architecture audit should choose a materially new storage interaction. Strong candidates are a virtio-blk completion interrupt path that composes the established PCI interrupt transports with real block completion ownership, or a write/durability phase only when its persistence/error semantics can be stated and tested honestly. Before broadening storage semantics, also revisit request-side guest-memory mutation atomicity: the first read slice validates descriptor/header/ring semantics before processing, but a future hardening phase may need an explicit whole-output-range preflight if failed multi-write DMA must be guaranteed non-partial. Performance work remains separate and requires controlled benchmark evidence.
