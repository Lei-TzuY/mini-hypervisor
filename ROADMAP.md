# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 loading/mapping, userspace and virtual MMIO, direct/controller-backed interrupt delivery, MMIO interrupt lifecycles and multi-device routing, host-driven timer delivery through direct `KVM_IRQ_LINE` and irqfd/eventfd, ioeventfd-backed device signaling, guest-discovered PCI BAR MMIO, bounded virtio-rng split-ring execution with INTx/MSI completion, bounded virtio-blk read/INTx execution, same-VM in-memory virtio-blk `T_OUT`→`T_IN` write/readback, bounded four-sector multi-sector read/write, and the bounded two-vCPU same-VM shared-memory handoff.

Current `main` is `f98d6344f25c4ac48b0eb8073be60cf1aa6185f4` through PR #97. The multi-sector milestone validates checked half-open block ranges against a four-sector in-memory backing and proves a guest-originated 1024-byte `T_OUT` followed by `T_IN` starting at sector 1 across the sector1→sector2 boundary while preserving untouched sectors. Its permanent hosted-KVM multi-sector workflow and the existing CI, virtio-blk INTx and one-sector write/readback workflows are green.

The fixed-capacity range-validation phase is sealed. Do not farm larger hard-coded capacities, additional payload signatures, or more fixed direct descriptor chains merely to extend the phase number.

## Selected milestone — negotiated virtio-blk indirect descriptor execution

The next storage/transport boundary is split-ring descriptor topology. This milestone promotes the bounded virtio-blk model from direct three-descriptor chains to negotiated `VIRTIO_RING_F_INDIRECT_DESC` execution while retaining the existing direct path as a compatibility contract and sharing one request resolver/processor.

This is a bounded indirect-descriptor proof, not full virtio conformance or arbitrary scatter/gather. The deterministic guest explicitly negotiates bit 28, submits indirect `T_OUT` and `T_IN` requests through one outer main-ring descriptor and a three-entry guest indirect table, and verifies the same write/readback semantics already established by the direct path.

Acceptance contract:

- preserve current main CI, the two-vCPU foundation, every integrated virtio-blk INTx/write-readback/multi-sector workflow, all existing long-mode/ELF64/MMIO/interrupt/PCI/virtio contracts, and Rust 1.74 MSRV;
- advertise `VIRTIO_RING_F_INDIRECT_DESC` as an optional device feature while preserving VERSION_1-only negotiation for existing direct guests;
- reject unsupported feature bits and reject an INDIRECT descriptor unless bit 28 was successfully negotiated before `FEATURES_OK`/`DRIVER_OK` execution;
- keep the direct three-descriptor request path as a compatibility path and route direct and indirect requests through the same bounded descriptor resolver and block request processor rather than duplicating T_IN/T_OUT semantics;
- accept an outer main-ring indirect descriptor only when its table length is a non-zero 16-byte multiple with at least the three descriptors required by the bounded block request and an entry count that fits the u16 descriptor-index space;
- reject invalid outer flag mixtures, guest-memory range failures, internal next indices outside the indirect table, descriptor cycles and nested `VIRTQ_DESC_F_INDIRECT` before block backing, guest output, status, used/avail indices, notify state or ISR state are mutated;
- preserve the outer main-ring descriptor id as the used-ring completion id even though the request body is resolved from the indirect table;
- deterministic model regressions must prove indirect `T_OUT`→`T_IN` round-trip semantics and prove negotiation/topology failures leave backing and queue/device state unchanged;
- deterministic executable guest uses indirect table GPA `0x18700`, negotiates driver features exactly `VIRTIO_F_VERSION_1 | VIRTIO_RING_F_INDIRECT_DESC` (`0x110000000`), and proves one write completion `{id=0,len=1,sector=0}` followed by one read completion `{id=0,len=513,sector=0}`;
- final used-ring state is exactly `{idx=2, first={id=0,len=1}, second={id=0,len=513}}`, request status is `VIRTIO_BLK_S_OK`, backing and guest readback equal the deterministic write payload, and no device event remains unconsumed;
- exact debug-port proof is `PIBWONRD`, where `I` proves the indirect feature negotiation path executed before the existing `B/W/O/N/R/D` write/readback proof;
- exact indirect execution accounting is 22 port-I/O exits, 26 MMIO exits and one terminal HLT. The direct compatibility path remains exactly 21 port-I/O exits, 22 MMIO exits and one HLT; the VM-exit budget must be topology-specific and equal those exact validated sequences rather than be widened arbitrarily;
- KVM-aware integration independently validates the negotiated feature mask, descriptor-table topology, completions, used-ring state, status, backing/readback, exact proof, exact exit counts and terminal architectural RFLAGS bit 1;
- stable CI must retain all existing permanent workflows and add an independent `Strict KVM virtio-blk indirect` workflow requiring features `0x110000000`, write completion `0/1/0`, read completion `0/513/0`, used state `2/0/1/0/513`, status `0`, proof `[80, 73, 66, 87, 79, 78, 82, 68]`, 22 port-I/O exits, 26 MMIO exits and terminal RFLAGS bit 1;
- feature negotiation, descriptor topology/range/cycle validation, atomic failure semantics, used-ring identity, exact exit accounting, proof, MSRV or real-KVM failures remain hard failures and must not be swallowed, skipped into success, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** add:

- packed rings, `EVENT_IDX`, arbitrary scatter/gather depth, nested indirect descriptors, request concurrency, multiple virtqueues or queue scheduling;
- persistent file/block backing, durability, fsync/flush semantics, cache/writeback policy, discard or write-zeroes;
- full virtio-blk conformance, arbitrary guest-driver interoperability, MSI-X expansion, PCI hotplug/bridges or ECAM expansion;
- DMA/IOMMU, concurrent SMP expansion, migration, resumable execution or whole-VM snapshots;
- throughput, latency or IOPS claims; CI execution time is not benchmark evidence.

## Promotion rule

After indirect descriptor execution is integrated and exact merged-`main` permanent workflows are green, seal the negotiated indirect split-ring proof rather than farming larger indirect tables or more fixed descriptor-chain shapes.

The next architecture audit should choose a materially higher storage/virtio frontier. Strong candidates are a real persistent backing layer with an explicit flush/durability and failure model, a packed-ring or `EVENT_IDX` phase only when it has a bounded executable guest and interoperability evidence, or a multi-queue/concurrency frontier only when ownership and ordering can be proven. Discard/write-zeroes, MSI-X expansion, DMA/IOMMU and performance remain separate higher-order phases.
