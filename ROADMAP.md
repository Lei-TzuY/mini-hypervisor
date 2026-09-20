# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `23c3917f96c0850f349aa859b0c15f381a34ec02` through PR #152 (`Reject checkpoints with pending accelerated doorbells`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; canonical versioned checkpoint schemas through exactly two quiescent virtio-blk devices; fixed two-registration acceleration ownership; canonical versioned registration-pair ownership; one canonical two-device outer transaction; transaction-coupled restored acceleration; independent two-device mutable write/readback continuity; and acceleration-aware checkpoint quiescence.

PR #152 sealed the single-vCPU host-acceleration/checkpoint ownership boundary. Its proof stops after KVM_IOEVENTFD has consumed one guest notify but before userspace services the eventfd, observes pending state `[true,false]`, requires checkpoint rejection without consuming the eventfd counter, consumes the preserved count through the normal path, services the real virtio-blk queue, delivers the matching irqfd, and only then permits exact capture at queues `[[1,1],[0,0]]`. Exact merged-main commit `23c3917f96c0850f349aa859b0c15f381a34ec02` completed all 52 push-triggered workflows successfully. Do not farm additional single-vCPU pending-doorbell variants.

## Selected milestone — two-vCPU full-controller checkpoint ownership

The next architectural boundary is coordinated multi-vCPU controller ownership. Existing `BoundedTwoVcpuCheckpoint` owns both vCPU register/special-register/MSR states plus the bounded page set, while existing full-controller checkpoints own the VM PIC/IOAPIC state and exactly one vCPU LAPIC. Those models are insufficient to compose safely with the two-device/acceleration transaction because a second vCPU's LAPIC would otherwise sit outside the checkpoint boundary.

Implementation continues on `milestone/two-vcpu-full-controller-checkpoint` from exact green `main=23c3917f96c0850f349aa859b0c15f381a34ec02`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI, the existing two-vCPU checkpoint proof, all controller/checkpoint regressions, and every applicable hosted-KVM workflow;
- require both vCPUs to be promoted to exact RUNNABLE MP state; use a deterministic userspace I/O marker only for synchronization, then re-enter KVM with guest single-step enabled so KVM retires the pending I/O and returns KVM_EXIT_DEBUG at the following RIP before the adjacent guard NOP executes; capture occurs only at that fully retired boundary rather than an outstanding KVM_EXIT_IO;
- compose the existing canonical two-vCPU page/register/special-register/MSR ownership with each vCPU's MP state, master PIC, slave PIC, IOAPIC, and one LAPIC snapshot for each owned vCPU;
- canonicalize vCPU/LAPIC ownership by vCPU id and reject binding to a different pair;
- capture exactly the existing bounded page ownership set `0x30000, 0x1fc000, 0x1fd000`;
- deliberately corrupt all three owned pages, both vCPU architectural states, both MP states, master PIC, slave PIC, IOAPIC, primary LAPIC, and secondary LAPIC, and require every component to independently mismatch;
- restore bounded page/vCPU state first and require it to verify exactly before mutating MP/controller state;
- then restore both MP states with exact KVM readback, followed by master PIC, slave PIC, IOAPIC, primary LAPIC, and secondary LAPIC, and require exact comparison of the complete ownership set;
- after exact restore, both RUNNABLE vCPUs must resume from those fully retired checkpoint boundaries and independently emit proofs `0` and `1`; completion evidence uses the same I/O-marker plus post-I/O debug boundary and does not rely on INIT/SIPI or HLT wakeups;
- add a dedicated proof binary, independent KVM integration test, and permanent hosted-KVM workflow.

## Scope boundary

This milestone establishes two-vCPU controller/checkpoint ownership only. It does not yet serialize a new versioned schema, attach virtio-blk devices or reconstructed host registrations, capture concurrent running vCPUs, migrate an in-flight interrupt, or claim atomic live migration across independently executing host threads.

## Promotion rule

After two-vCPU full-controller ownership is integrated and exact merged-`main` CI is green, seal this controller layer. The next executable frontier is to compose this dual-vCPU ownership with the existing two-device checkpoint transaction and acceleration-quiescence gate, so both producers and both device/host-registration lifecycles share one explicit quiescent transaction boundary before any later in-flight or external-storage durability work.
