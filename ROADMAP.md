# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `bff4fd7ec4032e35e536393190b7f8da88273c60` through PR #151 (`Prove dual-device write/readback after restored acceleration`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; canonical versioned checkpoint schemas through exactly two quiescent virtio-blk devices; fixed two-registration acceleration ownership; canonical versioned registration-pair ownership; one canonical two-device outer transaction; transaction-coupled restored acceleration; and independent mutable write/readback continuity for two restored virtio-blk devices.

PR #151 sealed the two-device mutable data-plane boundary. On the same exactly restored VM, each reconstructed accelerated device accepts a distinct deterministic sector-0 `T_OUT`, mutates only its own backing, then returns that exact payload through `T_IN`. Both queues advance independently from `0/0` to `2/2`, each device produces exactly two ioeventfd doorbells and two irqfd completions, final backings remain distinct, and the deterministic proof is `W0aMR0aXY1bNZ1bQD`. Exact merged-main commit `bff4fd7ec4032e35e536393190b7f8da88273c60` completed all 51 push-triggered workflows successfully. Do not farm additional sector numbers, payload patterns, or equivalent read/write variants.

## Selected milestone — acceleration-aware checkpoint quiescence

The next correctness boundary is host acceleration state that exists outside the semantic virtio device. KVM_IOEVENTFD can consume a guest MMIO notify and leave a positive eventfd counter before userspace bridges that notification into `MmioBus`. During that interval the virtio-blk device still reports `notify_pending == false`, so device-only quiescence is insufficient evidence that a checkpoint is safe: the guest queue may already advertise work whose only delivery token lives in an external host fd.

Implementation continues on `milestone/acceleration-aware-checkpoint-quiescence` from exact green `main=bff4fd7ec4032e35e536393190b7f8da88273c60`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow;
- do not serialize raw file descriptors or eventfd kernel state into the checkpoint schema;
- add a non-consuming readiness probe for reconstructed ioeventfd doorbells; probe failures must fail closed;
- expose fixed-pair quiescence evidence and reject acceleration-aware checkpoint capture whenever either doorbell is pending;
- require the guest vCPU to be stopped before the quiescence gate is used, so the probe and subsequent capture have a stable producer boundary;
- rejection must not consume, clear, service, or acknowledge the pending eventfd count;
- executable proof must submit exactly one first-device request through KVM_IOEVENTFD, stop after KVM consumed the MMIO notify but before userspace services the eventfd, and observe pending state `[true, false]`;
- the acceleration-aware two-device checkpoint path must reject that state while the existing semantic device still appears quiescent;
- immediately after rejection, the same pending state must remain observable and the normal `wait_doorbell` path must still consume exactly count 1;
- service that preserved request through the real restored-device bridge and atomic virtio-blk queue path, deliver its matching irqfd, and verify the first queue reaches `1/1` while the untouched second queue remains `0/0`;
- once both eventfds are non-readable and both devices are semantically quiescent, the same acceleration-aware capture path must succeed and record queue ownership `[[1,1],[0,0]]`;
- deterministic guest/host evidence must produce proof `P0aSD` and completion RFLAGS bit 1 plus IF;
- add focused unit coverage for the non-consuming eventfd probe, an independent KVM integration test, proof binary, and permanent hosted-KVM workflow.

## Scope boundary

This milestone is a **quiescence gate**, not in-flight migration. It does not checkpoint an eventfd counter, raw fd, partially executed virtio request, outstanding irqfd signal, third device, or multi-vCPU producer race. A pending accelerated notification is rejected and preserved for normal servicing; capture is allowed only after host acceleration and semantic device state are both quiescent.

## Promotion rule

After this gate is integrated and exact merged-`main` CI is green, seal the single-vCPU acceleration/checkpoint ownership boundary. The next architectural frontier should then be coordinated multi-vCPU checkpoint ownership around the same transaction/device surface, because producer quiescence becomes a first-class requirement once more than one vCPU can submit or observe work. Only after that boundary is explicit should the project revisit any bounded in-flight replay token or external-storage durability model.
