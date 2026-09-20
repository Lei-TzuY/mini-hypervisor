# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `334de69fd2ccab7b9f60c32cc09b951e38836438` through PR #155 (`Version the two-vCPU two-device checkpoint transaction`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; bounded SYSCALL/SYSRET and usercopy; isolated ring3 address spaces; dirty-page tracking; bounded scheduling/wait ownership; versioned single-vCPU/controller/device checkpoints; two-device acceleration reconstruction; transaction-coupled dual-device mutable write/readback replay; acceleration-aware checkpoint quiescence; exact two-vCPU controller ownership including MP states and both LAPICs; one runtime transaction owning both vCPUs, both devices and the fd-free registration pair; and a canonical v1 byte format for that complete bounded transaction.

PR #155 sealed the multi-vCPU wire-format boundary. Capture owns canonical vCPU ids `[0,1]`, both RUNNABLE MP states, the bounded page set, PIC/IOAPIC, both LAPICs, exactly two quiescent virtio-blk devices and a matching versioned registration pair. Its proof encodes the outer transaction, discards encoder-side semantic ownership, decodes and re-encodes byte-for-byte canonically, materializes only from decoded bytes, deliberately corrupts every owned layer, restores exactly, reconstructs acceleration only after restore, and resumes both producers. Exact merged-main commit `334de69fd2ccab7b9f60c32cc09b951e38836438` completed all 56 push-triggered workflows successfully. Do not add another schema envelope around the same ownership graph.

## Selected milestone — restored multi-producer mutable data-plane replay

The next executable boundary is cross-layer producer/device ownership after the canonical #155 byte boundary. The restored transaction must prove that each restored vCPU independently drives only its bound restored virtio-blk device through reconstructed acceleration while mutable backing state remains isolated.

Implementation continues on `milestone/restored-multi-producer-data-plane-replay` from exact green `main=334de69fd2ccab7b9f60c32cc09b951e38836438`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI, #151/#152/#153/#154/#155 regressions and every applicable hosted-KVM workflow;
- reuse the #155 versioned transaction transport; encode the runtime transaction, discard encoder-side ownership, decode/re-encode canonically, and materialize only from decoded bytes rather than introducing another schema;
- expand the bounded page ownership from three to five pages by including queue pages `0x18000` and `0x19000` alongside the shared page and both producer stack pages, so descriptor/avail/used/header/data/status bytes cross the same canonical wire boundary;
- capture exactly two queue-ready virtio-blk devices at queue indices `0/0` with distinct deterministic T_OUT payloads already resident in their owned queue pages;
- use a versioned registration pair whose doorbells remain bound to BAR notify addresses but whose irqfd GSIs are 16 and 17;
- checkpoint IOAPIC pin 16 as fixed vector `0x50` to physical APIC id 0 and pin 17 as fixed vector `0x51` to physical APIC id 1, avoiding legacy-PIC ambiguity;
- install per-vector handlers that validate and clear the matching virtio ISR, emit producer-specific handler evidence, issue LAPIC EOI, and return to the producer;
- both producers must stop at retired post-I/O capture barriers before transaction capture; acceleration must be quiescent `[false,false]`;
- deliberately corrupt all five owned pages, both architectural vCPU states, both MP states, PIC/IOAPIC, both LAPICs and both devices; require a real full mismatch including both queue pages;
- restore the materialized transaction exactly, reconstruct the registration pair against that same restored VM, require queue indices `0/0`, preserved IOAPIC routes and reconstructed pending state `[false,false]`;
- vCPU0 must issue only device0 sector-0 T_OUT then T_IN, with proof `W0aMR0aXD`; vCPU1 must issue only device1 T_OUT then T_IN, with proof `Y1bNZ1bQE`;
- every notify must be consumed by its matching ioeventfd, serviced by the atomic virtio-blk queue path against the same restored guest memory, and completed through its matching irqfd to the correct producer;
- each device must finish at queue `2/2` with exactly two doorbell events and two irqfd signals;
- final readback and backing for each device must equal only that producer's distinct original payload, and the two final backings must differ;
- reconstructed registrations must be deassigned on both success and replay failure;
- add a proof binary, independent KVM integration test and permanent hosted-KVM workflow with compilation outside the 30-second execution timeout.

## Scope boundary

This milestone is two restored producers, two restored devices, one write and one readback per producer/device pair, executed sequentially on one restored VM after a quiescent checkpoint. It does not claim concurrent running-vCPU capture, simultaneous queue submission, in-flight interrupt/queue migration, a third device, cross-host live migration, external-storage crash consistency, or performance/downtime properties.

## Promotion rule

After restored multi-producer mutable replay is integrated and exact merged-`main` CI is green, seal the quiescent two-producer/two-device cross-layer phase. The next architectural hypothesis should be a bounded in-flight ownership token only if the current virtio/acceleration model can state and validate exactly what may remain outstanding at capture; otherwise promote to a real backend/durability abstraction before making any external-storage migration claim.
