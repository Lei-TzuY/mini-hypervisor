# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `c871d551fbad5e90f38b4b82601d6f2c98df8d2b` through PR #150 (`Replay two restored devices through reconstructed acceleration`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; canonical versioned checkpoint schemas through exactly two quiescent virtio-blk devices; fixed two-registration acceleration ownership; canonical versioned registration-pair ownership; one canonical two-device outer transaction; and transaction-coupled replay on the same restored VM.

PR #150 sealed the read-only lifecycle coupling boundary. Its executable path composes the existing two-device checkpoint transaction, discards encoder-side ownership, decodes and materializes the checkpoint plus registration pair from canonical bytes, deliberately corrupts pages/VCPU/PIC/device state, restores exactly, reconstructs fresh ioeventfd/irqfd registrations against that same restored VM, and drives one sector-0 read through each restored virtio-blk queue. Both independent queues advance from `0/0` to `1/1`, both accelerated notifications are consumed by ioeventfd, both completions are delivered by their matching irqfd, and the proof `A0aMB1bND` is observed. Exact merged-main commit `c871d551fbad5e90f38b4b82601d6f2c98df8d2b` completed all 50 push-triggered workflows successfully. Do not farm extra read-only markers, GSI aliases or repeated envelope variants.

## Selected milestone — transaction-coupled dual-device write/readback continuity

The next executable boundary is mutable data-plane ownership. The same restored VM must prove that both reconstructed accelerated virtio-blk devices can independently accept a distinct sector-0 write, mutate only their own backing, and then return that exact payload through a subsequent read before completion.

Implementation continues on `milestone/transaction-coupled-dual-device-write-readback` from exact green `main=c871d551fbad5e90f38b4b82601d6f2c98df8d2b`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow green on the exact base;
- reuse the canonical two-device checkpoint transaction and reconstructed registration-pair lifecycle rather than introducing a parallel schema or synthetic device API;
- prepare exactly two queue-ready, quiescent virtio-blk devices at BARs `0x10000000` and `0x10001000`, with independent queue ownership in checkpoint pages `0x18000` and `0x19000`;
- capture both queues at exactly `0/0`, serialize the outer transaction, discard encoder-side ownership, decode/re-encode canonically, and materialize only from the byte stream;
- deliberately corrupt both owned pages, vCPU/PIC state and both device states, prove a real mismatch, then require exact checkpoint restore before acceleration reconstruction;
- reconstruct the decoded registration pair against the same restored VM;
- issue one `VIRTIO_BLK_T_OUT` request per restored device using distinct deterministic 512-byte payloads, require descriptor 0 / sector 0 / completion length 1 / status OK, and verify each backing mutates immediately to only its own payload;
- for each device, rewrite the queue request to `VIRTIO_BLK_T_IN`, overwrite the guest data buffer with a sentinel, submit a second accelerated notification, and require descriptor 0 / sector 0 / completion length 513 / status OK;
- each accelerated notify must be consumed by KVM_IOEVENTFD without a userspace notify event, queue service must occur against the same restored `MmioBus` and VM guest memory, and completion delivery must use the matching reconstructed irqfd on GSI 0/vector `0x40` and GSI 1/vector `0x41`;
- after both write/readback pairs, both queues must advance independently from `0/0` to `2/2`, with exactly two doorbell events and two irqfd signals per device;
- final guest readback and final device backing for each device must equal that device's original distinct write payload, and the two final backings must remain different to reject accidental cross-device aliasing;
- deterministic evidence must produce proof `W0aMR0aXY1bNZ1bQD` and completion RFLAGS bit 1 plus IF;
- registration-pair cleanup must run even if replay fails; queue processing, cleanup, restore, ordering, payload or proof failures remain hard failures;
- add focused unit coverage, an independent KVM integration test, proof binary and permanent hosted-KVM workflow with compilation outside the 30-second execution timeout.

## Scope boundary

This milestone is exactly two devices, one vCPU, one sector-0 write and one sector-0 readback per device after restore. It does **not** checkpoint an in-flight write, serialize raw file descriptors, add a third device, add multi-vCPU transaction ownership, claim cross-host/live migration compatibility, add external-storage crash consistency, or make performance/downtime claims.

The intent is to close the ownership gap left by #150: read-only accelerated replay proves transport and interrupt continuity, while distinct write/readback must prove that the restored data plane preserves independent mutable backing ownership across both reconstructed devices.

## Promotion rule

After transaction-coupled dual-device write/readback is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this two-device mutable data-plane phase rather than farming repeated sectors or payload variants.

The next architecture frontier should promote whichever real gap remains after this proof: a bounded in-flight queue/checkpoint ownership model if supported by the existing device semantics, coordinated multi-vCPU transaction ownership if queue state is already sealed, or external-storage durability/interoperability only when the repository has a real backend abstraction capable of supporting evidence. Controlled performance/observability work remains later.
