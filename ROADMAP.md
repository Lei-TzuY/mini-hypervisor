# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO execution, long-mode virtual-MMIO composition, direct and controller-backed interrupt delivery, MMIO-device interrupt lifecycles, bounded multi-device MMIO registration/mapping, dual-source legacy-PIC routing, host-driven timer delivery through direct `KVM_IRQ_LINE` and irqfd/eventfd, one ioeventfd-to-irqfd accelerated doorbell round trip, guest-discovered PCI BAR-backed MMIO, one modern virtio-rng PCI split-ring request, virtio-rng completion through bounded legacy INTx and guest-programmed MSI, and one bounded modern virtio-blk sector-read path.

The bounded virtio-blk read phase is integrated at commit `78ce397e587e6ef1adb0677b766ea5eeb6123a75` through PR #93. Exact merged-main CI #479 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all seventeen earlier strict real-KVM gates, and the eighteenth strict virtio-blk sector-read gate. That path discovers PCI/BAR/capabilities, negotiates `VIRTIO_F_VERSION_1`, executes one checked three-descriptor `VIRTIO_BLK_T_IN` request for sector 0, returns the deterministic 512-byte sector, publishes used `{ id=0, len=513 }`, writes `VIRTIO_BLK_S_OK`, and proves `PBNR` with exact exit accounting.

That polling/read-to-clear sector-read phase is sealed. Do not farm more fixed sectors, duplicate descriptor layouts, or another polling proof merely to extend the phase number.

## Selected milestone — virtio-blk completion through bounded legacy INTx

The next architecture boundary is block-completion ownership rather than another storage payload. This milestone composes the integrated virtio-blk queue/data path with the existing in-kernel irqchip, LAPIC ExtINT boundary and legacy-PIC interrupt lifecycle so a completed block request wakes the guest through one real controller-backed INTx path.

This is deliberately one bounded completion lifecycle for the already integrated sector-0 read. It is not a second block operation, MSI/MSI-X phase, write/durability claim, repeated-I/O path or general PCI interrupt-routing model.

Acceptance contract:

- preserve all eighteen integrated strict real-KVM gates and every existing long-mode, ELF64, MMIO, accelerated-event, PCI, virtio-rng, virtio-blk, interrupt, diagnostic and Rust 1.74 MSRV contract;
- keep the integrated polling `run_virtio_blk_pci_guest` behavior unchanged and reuse the same virtio-blk PCI function, transport state machine, deterministic backing sector and checked three-descriptor request processor;
- create the INTx VM through the existing in-kernel irqchip path and preserve software-enabled LAPIC SPIV with unmasked ExtINT LINT0;
- deterministic guest code must remap the master PIC to vectors `0x40..0x47`, unmask IRQ0, enable IF, discover the same virtio-blk PCI identity/BAR/capability chain, negotiate `VIRTIO_F_VERSION_1`, configure queue 0 and submit the same sector-0 `VIRTIO_BLK_T_IN` request;
- the notify MMIO exit remains serviceable/in-flight: guest must re-enter and emit the explicit `N` completion barrier before userspace consumes the source-tagged queue event and processes guest memory;
- userspace may assert GSI0 exactly once only after the `N` barrier and successful completion publication; completion state remains descriptor `0`, length `513`, sector `0`, used tuple `1/0/513`, status `VIRTIO_BLK_S_OK`, queue enabled and the full deterministic 512-byte sector;
- vector `0x40` handler must emit `I`, read the virtio ISR status, require the queue bit, emit `A`, issue master-PIC EOI and `iretq`;
- the ISR MMIO read remains serviceable/in-flight: userspace may deassert GSI0 only after the guest re-enters and emits post-ISR-read barrier `A`; assert and deassert counts must each be exactly one and no device event may remain pending;
- after interrupt return the mainline must validate used ring, request status, deterministic sector boundaries and cleared ISR state, emit `R`, then emit final userspace completion barrier `D`;
- exact debug-port proof is `PBNIARD`: `P/B/N` preserve the integrated discovery/queue/notify ordering, `I/A` prove entry into the INTx handler and committed ISR read-to-clear, `R` proves return to the interrupted mainline with block results intact, and `D` is the final userspace synchronization barrier;
- exact port-I/O accounting is twenty-one exits: seven PCI configuration read cycles plus seven one-byte debug outputs; exact MMIO accounting is twenty-two exits: the integrated twenty-one block accesses plus the handler ISR read-to-clear;
- final completion RFLAGS must retain architectural bit 1 and IF, and LAPIC SPIV/LINT0 semantics must remain valid;
- KVM-aware integration must independently validate GSI/vector, one assert/deassert lifecycle, completion/used/status/features/queue state, all 512 data bytes, exact proof, port-I/O/MMIO accounting, LAPIC state and completion RFLAGS;
- the permanent existing `CI` workflow must remain green with all eighteen integrated strict real-KVM gates unchanged;
- an independent permanent `Strict KVM virtio-blk INTx` workflow is the nineteenth executable gate and must require GSI0/vector `0x40`, one assert/deassert, features `0x100000000`, queue enabled, completion `0/513/0`, used `1/0/513`, status `0`, 512 data bytes, deterministic first-16/last-8 signatures, proof bytes `[80, 66, 78, 73, 65, 82, 68]`, twenty-one port-I/O exits, twenty-two MMIO exits, semantic LAPIC ExtINT state and completion RFLAGS bit1+IF;
- queue ownership, interrupt assertion/deassertion ordering, ISR read-to-clear semantics, guest-memory completion publication, exact proof/accounting, controller state or MSRV failures remain hard failures and must not be swallowed, skipped into success, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** add:

- `VIRTIO_BLK_T_OUT`, flush, discard, write-zeroes, barriers, persistence, filesystem semantics or any durability claim;
- more than the existing single sector-0 request, repeated requests, multiple queues, queue wraparound, indirect descriptors, event-index, packed rings or interrupt suppression;
- virtio-blk MSI/MSI-X, irqfd/ioeventfd acceleration for this device, arbitrary PCI routing, additional PIC lines, IOAPIC routing or x2APIC;
- arbitrary guest-driver compatibility, full virtio-blk conformance/interoperability, hotplug, PCI bridges, PCIe ECAM or BAR relocation/sizing;
- controlled storage benchmarks, throughput/latency claims, caching/writeback policy or host-file/block-device persistence;
- DMA/IOMMU infrastructure, SMP/multi-vCPU execution, migration, resumable execution or whole-VM snapshots.

## Promotion rule

After this INTx completion path is integrated and exact merged-`main` checks are green, seal the one-sector INTx composition rather than farming the same completion through another fixed vector or transport merely to increase the phase count.

The next architecture audit should prefer a materially new storage semantic. A strong next slice is a bounded in-memory `VIRTIO_BLK_T_OUT` mutation followed by an independently validated `T_IN` readback in the same VM, explicitly without persistence/durability claims. Before or within that expansion, revisit whole-output-range/request mutation preflight so failed multi-write guest-memory operations cannot create partially committed device results. A virtio-blk MSI completion variant is lower priority unless it unlocks a genuinely new controller or transport invariant rather than repeating the already proven virtio-rng MSI pattern. Performance remains a separate frontier and requires controlled benchmark evidence.
