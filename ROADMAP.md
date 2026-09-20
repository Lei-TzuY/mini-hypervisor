# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `db36f87df370120b2b732e6e5a764fe70591dcb4` through PR #154 (`Bind two vCPUs and two devices into one runtime checkpoint transaction`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; bounded SYSCALL/SYSRET and usercopy; isolated ring3 address spaces; dirty-page tracking; bounded scheduling/wait ownership; versioned single-vCPU/controller/device checkpoints; two-device acceleration reconstruction; transaction-coupled dual-device read and mutable write/readback replay; acceleration-aware checkpoint quiescence; exact two-vCPU controller ownership including MP states/LAPICs; and one runtime transaction that owns both vCPUs, both devices and the fd-free registration pair.

PR #154 sealed the multi-vCPU runtime ownership boundary. Both RUNNABLE producers stop at fully retired post-I/O debug boundaries; capture binds both architectural vCPU states, MP states, PIC/IOAPIC, both LAPICs, exactly two quiescent virtio-blk devices, and a matching fd-free registration pair. The proof deassigns capture-time acceleration, corrupts every checkpoint-owned layer, restores the dual-vCPU controller before atomically restoring devices, reconstructs fresh ioeventfd/irqfd registrations only after exact restore, re-verifies that reconstruction did not mutate semantic state, then resumes both producers. Exact merged-main commit `db36f87df370120b2b732e6e5a764fe70591dcb4` completed all 54 push-triggered workflows successfully. Do not farm additional unversioned two-vCPU/two-device variants.

## Selected milestone — versioned two-vCPU two-device checkpoint transaction

The next executable boundary is canonical byte ownership for the exact runtime model proven by #154. No encoder-side checkpoint object, device state or host-registration descriptor may survive across the wire boundary used by the proof.

Implementation continues on `milestone/versioned-two-vcpu-two-device-transaction` from exact green `main=db36f87df370120b2b732e6e5a764fe70591dcb4`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI, #153/#154 regressions, existing versioned single-vCPU/two-device schemas, acceleration-quiescence proof, and every applicable hosted-KVM workflow;
- version the standalone secondary vCPU register/special-register/MSR snapshot with explicit magic/version/architecture/length/count/flags/reserved fields, canonical MSR ordering, host-MSR compatibility validation, and fail-closed decode;
- version the two-vCPU full-controller checkpoint including canonical vCPU ids, both MP states, master/slave PIC, IOAPIC, and both LAPICs;
- version exactly two quiescent virtio-blk states around that controller checkpoint with canonical distinct aligned BAR ownership;
- add one outer transaction envelope binding the versioned two-vCPU/two-device checkpoint to the existing versioned host-registration pair;
- revalidate at encode, decode and materialize time that each registration doorbell matches its device BAR notify address and fixed queue-notify semantics;
- reject unsupported version/architecture, bad lengths/counts, non-zero reserved/flags, invalid MP state, noncanonical vCPU ids/BAR order/MSR order, incompatible host MSRs, invalid device state, and registration/device binding mismatches;
- runtime proof must capture through #154's exact quiescent transaction, encode it, discard encoder-side semantic ownership, decode and re-encode byte-for-byte canonically, materialize only from decoded bytes, then run the same deliberate full corruption → exact restore → registration reconstruction → dual-producer completion path;
- executable evidence must report transaction/checkpoint/controller/registration schema versions, canonical roundtrip, canonical vCPU ids `[0,1]`, MP states `[0,0]`, exactly three owned pages, per-vCPU MSR counts, BARs `0x10000000/0x10001000`, and 2048-byte backing ownership per device;
- after materialization the same runtime proof must still show capture/reconstructed doorbells `[false,false]`, mutation mismatch, exact restore, statuses `[1,3]`, capture RIPs `0x1000e/0x11006`, completion RIPs `0x10023/0x1101b`, and producer proofs `0` / `1`;
- add focused corruption tests for each new envelope layer, a dedicated proof binary, independent KVM integration test, and permanent hosted-KVM workflow.

## Scope boundary

This milestone creates a canonical v1 byte format for the already proven bounded runtime ownership model. It does not serialize raw file descriptors or eventfd counters, capture concurrently running vCPUs, migrate in-flight interrupts/queues, add a third device, claim cross-host live migration, or make external-storage durability/performance claims.

## Promotion rule

After the versioned transaction is integrated and exact merged-`main` CI is green, seal the multi-vCPU wire-format boundary. The next architectural frontier is restored multi-producer data-plane replay: each restored vCPU must independently drive its bound restored virtio-blk device through reconstructed acceleration while preserving device/backing isolation. Only after that cross-layer proof should the project consider bounded in-flight queue tokens or external-storage durability.
