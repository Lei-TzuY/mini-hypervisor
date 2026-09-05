# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO execution, long-mode virtual-MMIO composition, direct and controller-backed interrupt delivery, MMIO-device interrupt lifecycles, bounded multi-device MMIO registration/mapping, dual-source legacy-PIC routing, host-driven timer delivery through direct `KVM_IRQ_LINE` and irqfd/eventfd, one ioeventfd-to-irqfd accelerated doorbell round trip, guest-discovered PCI BAR-backed MMIO, one modern virtio-rng PCI split-ring request, virtio-rng completion through bounded legacy INTx and guest-programmed MSI, one bounded modern virtio-blk sector-read path, and virtio-blk completion through bounded legacy INTx.

The virtio-blk INTx phase is integrated at commit `d32685b5453c3d1ae86ff76d0beac2b4af47094f` through PR #94. Exact merged-main `CI` and the independent `Strict KVM virtio-blk INTx` workflow both completed successfully. The integrated path keeps the sector-0 `VIRTIO_BLK_T_IN` request, publishes used `{ id=0, len=513 }`, raises GSI0 only after the explicit post-notify barrier, enters vector `0x40`, consumes the ISR queue bit, deasserts only after the committed ISR-read barrier, returns through `iretq`, and proves `PBNIARD` with exact exit accounting.

That one-sector INTx composition is sealed. Do not farm another fixed vector, another identical read, or an MSI clone merely to extend the phase number.

## Selected milestone — bounded in-memory virtio-blk write then readback

The next architecture boundary is mutable block storage semantics and whole-request commit ordering. This milestone adds one bounded `VIRTIO_BLK_T_OUT` mutation of sector 0, then submits an independent `VIRTIO_BLK_T_IN` through the same queue and same device instance in the same VM to prove the mutation is observable through the normal read path.

This is deliberately an in-memory device-state proof. It is not a persistence, durability, filesystem, cache-policy, host-file or host-block-device claim.

Acceptance contract:

- preserve all integrated main CI and virtio-blk INTx checks, every existing long-mode/ELF64/MMIO/interrupt/PCI/virtio-rng/virtio-blk contract, and Rust 1.74 MSRV;
- add bounded `VIRTIO_BLK_T_OUT` request type `1` for sector 0 while keeping `VIRTIO_BLK_T_IN` type `0` behavior intact;
- keep the existing three-descriptor request shape: header descriptor is device-readable with `NEXT`; T_OUT data is device-readable with `NEXT` and no `WRITE`; T_IN data remains `NEXT|WRITE`; status remains one-byte `WRITE`;
- T_OUT consumes exactly 512 guest bytes into owned host state, returns `VIRTIO_BLK_S_OK`, publishes used length `1`, and mutates only the existing in-memory sector-0 backing;
- T_IN continues to return the current 512-byte backing plus status and publishes used length `513`;
- before any guest output, backing-sector mutation, queue-index mutation, notify clear or ISR mutation, the processor must complete all fallible request reads/address arithmetic and preflight every guest range it will write; a later invalid status/used/data output must therefore fail without partially committing earlier output or device backing;
- deterministic unit regression must prove T_OUT followed by T_IN on the same `VirtioBlkDevice` returns the written bytes and advances avail/used state from 0→1→2;
- deterministic failure regressions must prove an out-of-bounds later used-ring range leaves T_OUT backing, T_IN data/status, queue indices, notify state and ISR state unchanged;
- runtime must process both requests through one registered virtio-blk MMIO device and the same queue, not through a shadow device or a second storage path;
- deterministic write payload must differ from the integrated read-only sector and expose stable first-16 `BLK-WRITE-0000!!` and last-8 `WRTBACK!` signatures;
- guest must submit T_OUT first, re-enter after notify and emit explicit write barrier `W`; userspace processes the queue only at that barrier and immediately verifies the backing mutation;
- guest must validate first completion used tuple `{ idx=1, id=0, len=1 }` and status `0`, emit `O`, overwrite the data buffer with a sentinel, rewrite the descriptor/header for T_IN, increment avail to 2, notify again and emit read barrier `N`;
- after atomic T_IN processing, guest must validate used tuple `{ idx=2, second id=0, second len=513 }`, status `0`, and both readback signatures, then read the ISR queue bit, emit `R`, emit final barrier `D`, and HLT;
- exact debug-port proof is `PBWONRD`; exact host-visible final backing and guest readback must both equal the deterministic write payload;
- KVM-aware integration must independently validate both completions, both used entries, status, backing, readback, exact proof, 21 port-I/O exits, 22 MMIO exits and terminal architectural RFLAGS bit 1;
- an independent permanent `Strict KVM virtio-blk write/readback` workflow must prove the same executable state on hosted KVM while the existing main CI and `Strict KVM virtio-blk INTx` workflow remain unchanged and green;
- descriptor direction, request ordering, guest-memory preflight, backing mutation, used/status publication, exact proof/accounting or MSRV failures remain hard failures and must not be swallowed, skipped into success, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** add:

- persistence, durability, fsync/flush semantics, host files, host block devices, caching/writeback policy, barriers, discard or write-zeroes;
- sectors beyond the existing one-sector capacity, multiple queues, queue wraparound beyond the bounded two requests, indirect descriptors, event-index or packed rings;
- virtio-blk MSI/MSI-X, additional interrupt transports, new PIC lines, IOAPIC routing, x2APIC or another completion mechanism;
- arbitrary guest-driver interoperability or full virtio-blk conformance;
- PCI hotplug, bridges, ECAM, dynamic BAR sizing/relocation, DMA/IOMMU, SMP, migration, resumable execution or whole-VM snapshots;
- throughput/latency/IOPS claims or uncontrolled benchmark evidence.

## Promotion rule

After the T_OUT→T_IN same-VM readback is integrated and exact merged-main checks are green, seal the single-sector in-memory read/write semantic rather than farming more payload patterns or repeated fixed-sector requests.

The next architecture audit should select a genuinely higher storage frontier: a bounded flush/durability model only if it can be tied to a real persistence backend and explicit failure semantics, a larger/multi-sector backing model with coherent range validation, or a transport/controller frontier that unlocks materially new behavior. Performance remains separate and requires controlled benchmark evidence.
