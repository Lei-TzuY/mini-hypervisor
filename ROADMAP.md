# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 loading/mapping, userspace and virtual MMIO, direct/controller-backed interrupt delivery, MMIO interrupt lifecycles and multi-device routing, host-driven timer delivery through direct `KVM_IRQ_LINE` and irqfd/eventfd, ioeventfd-backed device signaling, guest-discovered PCI BAR MMIO, bounded virtio-rng split-ring execution with INTx/MSI completion, bounded virtio-blk read/INTx execution, one same-VM in-memory virtio-blk `T_OUT`→`T_IN` write/readback, and the first bounded two-vCPU same-VM shared-memory handoff.

The single-sector mutable virtio-blk phase is integrated at commit `5b51cb340ea133b9478ea05e192114c4e1c91b6d` through PR #95. Its permanent hosted-KVM write/readback workflow proves one sector-0 `T_OUT` completion `{id=0,len=1}`, an independent `T_IN` completion `{id=0,len=513}`, exact backing/readback equality, proof `PBWONRD`, 21 port-I/O exits and 22 MMIO exits. This remains an in-memory semantic proof; it does not claim persistence or durability.

Current `main` is `c36869ed7c0d0e4d811a3db962e229e9ad6aaf37` through the disjoint PR #96 two-vCPU foundation. That milestone adds a deterministic sequential two-vCPU shared-memory handoff without claiming concurrent SMP, AP startup, IPIs or memory-ordering guarantees. Its merged-main checks and the existing virtio-blk workflows are green.

The single-sector storage mutation phase is sealed. Do not farm more sector-0 payload variants or repeated one-sector requests merely to extend the phase number.

## Selected milestone — bounded multi-sector virtio-blk read/write

The next storage boundary is backing-range architecture rather than another fixed payload. This milestone expands the deterministic in-memory virtio-blk backing to four 512-byte sectors, validates checked sector ranges, and proves a guest-originated 1024-byte `T_OUT` followed by `T_IN` starting at sector 1 across the sector1→sector2 boundary in one VM, one device and one queue.

This remains an in-memory block-device model. It is not a host-file, host-block-device, persistence, durability, flush, cache-policy or filesystem claim.

Acceptance contract:

- preserve current main CI, the two-vCPU foundation gate, the integrated virtio-blk INTx gate, the single-sector write/readback gate, every existing long-mode/ELF64/MMIO/interrupt/PCI/virtio contract, and Rust 1.74 MSRV;
- expose exactly four 512-byte sectors through the bounded device capacity while preserving existing sector-0 request behavior;
- accept only non-zero request lengths that are exact sector-size multiples and compute request backing ranges with checked half-open arithmetic before mutation;
- reject starting sectors or ranges outside capacity before modifying backing data, guest output, request status, avail/used indices, notify state or ISR state;
- `T_OUT` and `T_IN` must operate on the validated requested backing range rather than a hard-coded sector-0 slice;
- deterministic host/model regression must prove a 1024-byte `T_OUT`→`T_IN` round trip across sectors 1–2, while sector0 and sector3 remain byte-for-byte unchanged;
- deterministic failure regressions must prove invalid length/range failures leave backing, request status, used-ring indices, notify state and ISR state unchanged;
- executable guest must submit a two-sector `T_OUT` starting at sector1, commit it through the existing queue/device, then submit an independent two-sector `T_IN` through the same queue/device and receive exact readback across the sector boundary;
- T_OUT publishes status `VIRTIO_BLK_S_OK` and used length `1`; T_IN publishes status `VIRTIO_BLK_S_OK` and used length `1025`; final used ring state is exactly two entries with `{id=0,len=1}` then `{id=0,len=1025}`;
- capacity observed by the guest/device contract is exactly four sectors;
- deterministic payload signatures must verify the beginning, a signature crossing the 512-byte sector boundary, and the final bytes for both host backing and guest readback;
- untouched sector0 and sector3 state must be verified after both requests complete;
- exact debug-port proof is `PBWONRD`; KVM-aware integration independently validates request range, both completions, used-ring tuples, status, untouched sectors, signatures, exact proof, 21 port-I/O exits, 22 MMIO exits and terminal architectural RFLAGS bit 1;
- the existing one-sector permanent write/readback workflow and virtio-blk INTx workflow must remain green, while a separate permanent `Strict KVM virtio-blk multi-sector` workflow proves the new executable on hosted `/dev/kvm`;
- the strict multi-sector workflow must require range `sector=1 length=1024 capacity=4`, write completion `0/1/1`, readback completion `0/1025/1`, used state `2/0/1/0/1025`, status `0`, `sector0=true sector3=true`, identical host/readback boundary signatures, proof `[80,66,87,79,78,82,68]`, 21 port-I/O exits, 22 MMIO exits and terminal RFLAGS bit 1;
- range arithmetic, descriptor direction, guest-memory preflight, backing mutation, used/status publication, untouched-sector preservation, proof/accounting or MSRV failures remain hard failures and must not be swallowed, skipped into success, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** add:

- persistent file/block backing, durability, fsync/flush semantics, caching/writeback policy, barriers, discard or write-zeroes;
- capacity beyond the bounded four-sector model, arbitrary request sizes, request batching/concurrency, multiple queues or queue scheduling;
- indirect descriptors, event-index, packed rings, virtio-blk MSI/MSI-X or new completion transports;
- full virtio-blk conformance, arbitrary guest-driver interoperability or a filesystem/storage stack;
- PCI hotplug/bridges/ECAM expansion, DMA/IOMMU, concurrent SMP, migration, resumable execution or whole-VM snapshots;
- throughput, latency or IOPS claims; CI execution time is not benchmark evidence.

## Promotion rule

After multi-sector read/write is integrated and exact merged-`main` checks are green, seal the four-sector/range-validation proof rather than farming larger fixed capacities or more payload signatures.

The next architecture audit should choose a materially higher storage frontier. Strong candidates are a real persistent backend plus an explicit flush/durability failure model, indirect-descriptor support tied to an executable virtio request, or a multi-queue/concurrency frontier only when ownership and ordering can be proven. Discard/write-zeroes, packed rings, MSI-X, DMA/IOMMU and performance remain separate higher-order phases.
