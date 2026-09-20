# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `7d3eff9569a73ede7332a99e803d5880dca18c9f` through PR #149 (`Bind two-device checkpoint into one transaction`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; canonical versioned checkpoint schemas through exactly two quiescent virtio-blk devices; fixed two-registration acceleration ownership; canonical versioned registration-pair ownership; and one canonical two-device outer transaction.

PR #149 sealed the outer byte composition boundary. Its executable path encodes the two-device checkpoint and registration pair into one stream, discards encoder-side checkpoint/transaction objects, decodes/materializes both nested objects, proves exact two-device checkpoint restore and separately proves the decoded registration pair through fresh acceleration. Exact merged-main commit `7d3eff9569a73ede7332a99e803d5880dca18c9f` completed all 49 push-triggered workflows successfully, including ordinary `CI`, Rust 1.74 MSRV, the strict two-device transaction proof and every applicable existing KVM regression. Do not farm more envelope variants or corruption offsets.

## Selected milestone — transaction-coupled restored acceleration and dual-device replay

The next executable boundary is cross-layer lifecycle coupling. The decoded registration pair must be reconstructed around the **same VM instance** whose two-device checkpoint was materialized and exactly restored, and both restored virtio-blk devices must then complete independent post-restore queue requests through ioeventfd -> device queue processing -> irqfd delivery.

Implementation continues on `milestone/transaction-coupled-dual-device-replay` from exact green `main=7d3eff9569a73ede7332a99e803d5880dca18c9f`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow green on the exact base;
- prepare exactly two queue-ready, quiescent virtio-blk devices at BARs `0x10000000` and `0x10001000`, with independent queue ownership in checkpoint pages `0x18000` and `0x19000`;
- capture both queue pages, vCPU/controller state and both device states while both queues are exactly `0/0` and no notification is in flight;
- compose the existing `VersionedTwoDeviceCheckpointTransactionV1`, discard encoder-side ownership, decode/re-encode canonically and materialize both the checkpoint and registration pair only from the byte stream;
- deliberately corrupt both owned pages, vCPU/PIC state and both device states, prove a real mismatch, then require exact transaction checkpoint restore before any acceleration is reconstructed;
- reconstruct the decoded registration pair against that same restored KVM VM, not an independent acceleration fixture;
- issue one post-restore sector-0 read on each restored device using the existing two-byte zero doorbells, with guest queue state owned by the restored checkpoint pages;
- each accelerated notify must be consumed by KVM_IOEVENTFD without a userspace notify event, then bridged through `apply_virtio_blk_host_notification` and `process_virtio_blk_notification` against the same restored `MmioBus` and VM guest memory;
- completion delivery must use the matching reconstructed irqfd independently on GSI 0/vector `0x40` and GSI 1/vector `0x41`, with distinct guest handlers and PIC EOI;
- after replay, both queues must advance independently from `0/0` to `1/1`, both completions must be descriptor 0 / sector 0 / length 513 / status OK, both readback buffers must match their restored deterministic backing, and both device states must be quiescent;
- deterministic replay evidence must retain exactly one doorbell event and one irqfd signal per device, proof `A0aMB1bND`, and completion RFLAGS bit 1 plus IF;
- registration-pair cleanup must run even if replay fails; queue processing, cleanup, restore, event ordering or proof failures remain hard failures and must not be swallowed or rewritten into success;
- add focused unit coverage, an independent KVM integration test, proof binary and permanent hosted-KVM workflow with compilation outside the 30-second execution timeout.

## Scope boundary

This milestone is exactly two devices, one vCPU and one read request per device after restore. It does **not** checkpoint an in-flight request, serialize raw file descriptors, add a third device, add multi-vCPU transaction ownership, claim cross-host/live migration compatibility, add external-storage crash consistency, or make performance/downtime claims.

The intent is to close the architectural gap left by #149: checkpoint restore and reconstructed acceleration now have to meet inside one runtime instead of succeeding in two separate fixtures.

## Promotion rule

After transaction-coupled dual-device replay is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this lifecycle rather than farming extra markers, GSI aliases or repeated read-only requests.

The next architecture frontier should move the data plane forward: prove transaction-coupled **write/readback continuity** across both restored devices (including distinct backing mutation and replay evidence), or—if that exposes a more fundamental ownership gap—promote the bounded queue/backing abstraction needed to support it. Broader migration, coordinated multi-vCPU transaction ownership, external-storage durability and controlled performance work remain later phases.
