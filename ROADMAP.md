# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `a3acab9b0db32ea375a9a3c1a7f3cd8f0e5988db` through PR #148 (`Version the two-registration acceleration pair`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; canonical versioned checkpoint schemas through exactly two quiescent virtio-blk devices; one canonical one-device outer checkpoint transaction; fixed two-registration acceleration ownership; and a canonical versioned fd-free registration-pair schema.

PR #148 sealed the two-registration byte boundary. Its executable path constructs the canonical bytes, discards the encoder-side semantic pair, decodes/materializes from the byte stream and passes only the decoded pair into the already-proven two-generation ioeventfd/irqfd acceleration core. Exact merged-main commit `a3acab9b0db32ea375a9a3c1a7f3cd8f0e5988db` completed all 47 push-triggered workflows successfully, including ordinary `CI`, Rust 1.74 MSRV, the strict registration-pair proof and all applicable existing KVM regressions. Do not farm alternate pair framing, more corruption offsets, extra generations or GSI aliases.

## Selected milestone — canonical two-device checkpoint transaction

The next composition boundary is one canonical outer byte stream that binds the sealed `VersionedFullControllerTwoVirtioBlkCheckpointV1` and `VersionedHostRegistrationPairV1`. Both decoded nested objects must be executably consumed: the decoded checkpoint must drive the existing two-device corruption -> exact restore -> `ASBJMD` proof, and the decoded registration pair must drive the existing fresh two-generation acceleration proof.

Implementation continues on `milestone/two-device-checkpoint-transaction` from exact green `main=a3acab9b0db32ea375a9a3c1a7f3cd8f0e5988db`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow green on the exact base;
- define one canonical little-endian v1 outer transaction with explicit magic, version, x86-64 architecture identifier, fixed header length, total length, exact nested checkpoint length, exact fixed registration-pair length, zero flags and zero reserved state;
- reuse the sealed two-device checkpoint schema and sealed registration-pair schema verbatim rather than duplicate controller/device/registration semantics;
- reject bad magic/version/architecture/header/total/nested lengths, non-zero flags/reserved state, arithmetic overflow, truncation and all nested-schema corruption fail-closed;
- executable transport must encode the captured two-device checkpoint and canonical registration pair into one outer stream, discard the process-local checkpoint and encoder-side transaction object, decode only from bytes, require byte-for-byte canonical re-encode identity, then materialize both nested objects;
- the decoded checkpoint must continue through the existing real-KVM deliberate page/VCPU/master-PIC/slave-PIC/IOAPIC/LAPIC + both-device mismatch, exact restore, statuses `0x01`/`0x03`, `ASBJMD` proof and IF-set completion;
- the decoded registration pair must be passed into the existing two-generation acceleration core and preserve doorbells `0x10000100`/`0x10001100`, GSI 0/1, vectors `0x40`/`0x41`, event counts `[[1,1],[1,1]]`, proof `RA0MB1NCE0PF1QD` and IF-set completion;
- deterministic unit coverage must prove outer framing failure paths while nested schema suites remain authoritative for their own semantic corruption;
- add a dedicated real-KVM integration test, proof binary and permanent hosted-KVM workflow with proof build outside the 30-second execution timeout;
- schema, materialization, restore, registration reconstruction, cleanup, routing, ioeventfd/irqfd delivery or proof failures remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** claim that host acceleration is reconstructed inside the same VM instance that performed the checkpoint restore. The two decoded components are both executably consumed, but the registration-pair proof runs through the existing independent deterministic acceleration fixture. It also does not serialize raw file descriptors, add a third device or registration, checkpoint in-flight requests, add another vCPU, claim cross-host/live migration compatibility, add external-storage crash consistency or make performance/downtime claims.

## Promotion rule

After the canonical two-device transaction is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal the outer framing rather than farming envelope variants.

The next architecture frontier is **transaction-coupled restored acceleration and dual-device request replay**: reconstruct the decoded registration pair around the restored two-device runtime itself, drive independent post-restore request activity for both devices, and prove queue/backing/interrupt continuity without relying on a separate acceleration fixture. Only after that cross-layer lifecycle is executable should broader migration, multi-vCPU transaction ownership, external-storage durability or performance work be promoted.
