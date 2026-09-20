# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `76b1940a531ff663b1a03c2df14168d5cc803ab6` through PR #153 (`Checkpoint two vCPUs with complete controller ownership`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; canonical versioned checkpoint schemas through exactly two quiescent virtio-blk devices; fixed two-registration acceleration ownership; canonical versioned registration-pair ownership; one canonical two-device outer transaction; transaction-coupled restored acceleration; independent two-device mutable write/readback continuity; acceleration-aware checkpoint quiescence; and exact two-vCPU controller ownership including both MP states and both LAPICs.

PR #153 sealed the two-vCPU controller layer. Both RUNNABLE vCPUs stop only after their synchronization I/O has been retired through a post-I/O single-step debug boundary; capture owns the bounded page set, both architectural vCPU states, both MP states, master/slave PIC, IOAPIC, and one LAPIC per canonical vCPU id. The proof corrupts every owned component independently, restores exact state, and resumes both producers without INIT/SIPI or HLT wakeup dependence. Exact merged-main commit `76b1940a531ff663b1a03c2df14168d5cc803ab6` completed all 53 push-triggered workflows successfully. Do not farm more two-vCPU controller-only variants.

## Selected milestone — two-vCPU two-device runtime checkpoint transaction

The next architectural gap is the split between #153's dual-producer ownership and the existing two-device/host-acceleration transaction. The existing two-device checkpoint still owns only one vCPU, so the repository does not yet have one runtime boundary that simultaneously owns both producers, both LAPIC/MP states, both virtio-blk devices, and the fd-free semantics needed to reconstruct both ioeventfd/irqfd registrations.

Implementation continues on `milestone/two-vcpu-two-device-transaction` from exact green `main=76b1940a531ff663b1a03c2df14168d5cc803ab6`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI, PR #153's two-vCPU full-controller proof, the existing two-device transaction/replay/write-readback proofs, acceleration-quiescence proof, and every applicable hosted-KVM workflow;
- add one runtime checkpoint that composes `BoundedTwoVcpuFullControllerCheckpoint` with exactly two canonical quiescent virtio-blk devices;
- bind the fd-free host-registration pair to those same device BAR notify addresses and reject a mismatched registration pair before capture;
- require both vCPUs to be RUNNABLE and stopped at fully retired post-I/O debug boundaries before capture;
- require reconstructed ioeventfd doorbells to be non-readable `[false,false]` before the runtime transaction can capture;
- capture exactly pages `0x30000,0x1fc000,0x1fd000`, both vCPU architectural states, both MP states, PIC/IOAPIC, both LAPICs, and two distinct device states;
- deassign the capture-time host registrations, then deliberately corrupt every checkpoint-owned layer: all pages, both vCPU states, both MP states, PIC/IOAPIC, both LAPICs, and both virtio-blk devices; require independent mismatch evidence for each;
- restore the dual-vCPU/controller layer exactly before atomically restoring either device;
- reconstruct the registration pair only after exact checkpoint restore, require it to remain quiescent, and re-verify that reconstruction itself did not mutate restored semantic checkpoint state;
- resume both restored vCPUs from the same transaction boundary and independently emit proofs `0` and `1`;
- every error path after host-registration reconstruction must deassign both ioeventfd/irqfd registrations rather than leak process-local acceleration state;
- add a dedicated proof binary, independent KVM integration test, and permanent hosted-KVM workflow.

## Scope boundary

This milestone is runtime ownership composition, not a new wire format. It does not introduce a versioned two-vCPU transaction schema, serialize raw file descriptors/eventfd counters, capture concurrently running vCPUs, replay in-flight interrupts, add a third device, or claim live migration/external-storage durability.

## Promotion rule

After this runtime transaction is integrated and exact merged-`main` CI is green, seal the multi-vCPU runtime ownership boundary. The next executable frontier is a versioned two-vCPU two-device transaction schema that preserves this proven ownership model across bytes; only after that schema round-trips canonically should the project extend restored data-plane replay so each restored producer independently drives its bound device through reconstructed acceleration.
