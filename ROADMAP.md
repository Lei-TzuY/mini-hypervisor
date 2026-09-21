# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `edfc2d8208aa7f23901b15ea1cde6bd3fae2ca6a` through PR #158 (`Own drained virtio-blk notifications before queue service`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; bounded SYSCALL/SYSRET and usercopy; isolated ring3 address spaces; dirty-page tracking; bounded scheduling/wait ownership; versioned single-vCPU/controller/device checkpoints; two-device acceleration reconstruction; transaction-coupled mutable replay; acceleration-aware checkpoint quiescence; exact two-vCPU controller/MP/LAPIC ownership; one runtime transaction owning both vCPUs, both devices and the fd-free registration pair; restored multi-producer mutable data-plane replay; a v2 serviced-completion token; and a v3 drained-notification token.

PR #158 sealed the in-memory notification/completion ownership chain. Its executable proof drains one real ioeventfd request, materializes `notify_pending=true` without queue service, captures a canonical fd-free v3 token at queues `[[0,0],[0,0]]`, discards encoder ownership, restores exact state from decoded bytes, services the restored request without a duplicate write doorbell, delivers the reconstructed irqfd, and completes ordinary readback with proof `W0aMR0aXD`. Exact merged-main commit `edfc2d8208aa7f23901b15ea1cde6bd3fae2ca6a` completed all 58 push-triggered workflows successfully. Queue service itself is atomic in the current model, so do not manufacture partially-serviced token variants.

## Selected milestone — synced file-backed virtio-blk storage boundary

The next architectural gap is storage ownership. The current virtio-blk implementation embeds a deterministic four-sector byte array directly in each device and serializes that array into checkpoint state. There is no host-file backend, no persistence boundary and no evidence that a completed guest write survives device reconstruction.

Implementation continues on `milestone/file-backed-virtio-blk` from exact green `main=edfc2d8208aa7f23901b15ea1cde6bd3fae2ca6a`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, all existing in-memory virtio-blk behavior, checkpoint/transaction regressions and permanent workflows;
- keep the existing four-sector in-memory backing as the default path;
- add one concrete file-backed mode backed by an exact `VIRTIO_BLK_BACKING_SIZE` host file; creation initializes deterministic contents and synchronizes them before the device is returned;
- opening an existing file-backed device must reject a backing whose size is not exactly the bounded capacity and must load the current bytes as the device cache;
- a file-backed `VIRTIO_BLK_T_OUT` must finish all guest/input/output preflight first, write the requested host-file range, and successfully call `sync_all` before guest status/used-ring/ISR completion becomes visible;
- if host backing write or sync fails, queue indices, ISR and guest completion must not be advanced;
- only after host sync succeeds may the in-memory cache and virtqueue completion be committed;
- executable evidence must write a deterministic sector-0 payload through the real atomic virtio-blk queue path, inspect the raw host file, drop the device, reopen a new device from that file, and read the same sector through a real `T_IN` queue request;
- final persisted sector and guest readback must match exactly, with write completion length 1 and read completion length 513;
- current checkpoint schemas must fail closed for file-backed devices because they do not encode external storage identity; no checkpoint may silently downgrade a file-backed device into an in-memory device;
- malformed file length must fail before the device is opened for guest service;
- add focused backend/unit coverage, an independent integration test, proof binary and permanent workflow.

## Scope boundary

This milestone proves a bounded **write + `sync_all` + drop/reopen + readback** contract on a normal host file. It does not claim power-loss atomicity, filesystem crash consistency, ordered metadata persistence, direct I/O, sparse-file semantics, concurrent external writers, cross-host migration, snapshotting of external storage identity, or production-grade storage performance.

The file path remains host-local runtime configuration and is deliberately excluded from the existing checkpoint byte formats. External mutation while a device is open is outside this milestone.

## Promotion rule

After the synced file-backed path is integrated and exact merged-`main` CI is green, seal the basic external-storage boundary. The next audit should choose between explicit checkpoint/storage identity coordination (only if a stable backend identity contract can be defined without serializing host-specific raw handles), bounded write-failure recovery/fault injection, or another cross-layer storage capability. Do not claim crash consistency without controlled failure evidence.
