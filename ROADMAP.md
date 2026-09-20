# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `e21b7332a3858932cea671e36d9a92ce4b649e39` through PR #147 (`Reconstruct two host-acceleration registrations`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; canonical versioned checkpoint schemas through exactly two quiescent virtio-blk devices; one canonical one-device outer checkpoint transaction; and bounded fd-free host-registration reconstruction for one descriptor and one fixed canonical runtime pair.

PR #147 sealed two-device host-acceleration lifecycle ownership. The pair is canonical by doorbell address, rejects overlapping doorbells and duplicate GSIs, rolls back the first registration if the second cannot be reconstructed, attempts reverse cleanup for both members, and proves two independent GSI/vector paths across two fresh ioeventfd/irqfd generations. Exact merged-main commit `e21b7332a3858932cea671e36d9a92ce4b649e39` completed all 46 push-triggered workflows successfully, including ordinary `CI`, MSRV and the strict two-host-registration acceleration proof. Do not farm additional generations, GSI aliases or doorbell variants.

## Selected milestone — canonical versioned host-registration pair

The next nested ownership boundary is the byte representation of the already-proven runtime pair. A single `HostRegistrationSpec` already has a sealed v1 fd-free schema; this milestone composes exactly two of those existing nested schemas into a canonical pair envelope and proves that acceleration is reconstructed from the decoded pair rather than from the original process-local values.

Implementation continues on `milestone/versioned-host-registration-pair` from exact green `main=e21b7332a3858932cea671e36d9a92ce4b649e39`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow green on the exact base;
- define one canonical v1 fd-free pair envelope with explicit magic, version, fixed header length, fixed total length, exact member count two, two exact nested lengths, zero flags and zero reserved fields;
- reuse two existing `VersionedHostRegistrationSpecV1` payloads verbatim rather than duplicating doorbell address, length, datamatch or GSI fields;
- canonical nested order is strictly ascending doorbell address; a byte stream with reversed members must fail closed rather than be silently normalized;
- nested standalone validation remains authoritative for each descriptor, then pair decode must re-run non-overlap and unique-GSI invariants before materialization;
- wrong magic/version/header/total/count/member lengths, non-zero flags/reserved bytes, nested-schema corruption, truncation, reversed order, overlapping ranges and duplicate GSI ownership all remain explicit failures;
- canonical encode→decode→re-encode identity is required;
- executable proof must encode the canonical pair, decode/materialize it, and pass only that decoded pair into the already-sealed two-generation acceleration core;
- the decoded-pair KVM proof must preserve doorbells `0x10000100`/`0x10001100`, GSI 0/1, vectors `0x40`/`0x41`, event counts `[[1,1],[1,1]]`, proof `RA0MB1NCE0PF1QD`, watchdog non-intervention and IF-set completion;
- deterministic unit tests must cover canonical roundtrip, outer framing corruption, nested corruption, truncation, reversed member order, pair overlap and duplicate GSI;
- add a dedicated integration test, proof binary and permanent hosted-KVM workflow whose build occurs outside the 30-second execution timeout;
- schema, semantic validation, reconstruction, cleanup, routing, ioeventfd/irqfd delivery or proof failures remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** widen the outer checkpoint transaction, serialize raw file descriptors or kernel registration identities, add a third descriptor, change the two-device checkpoint schema, process full virtio-blk request data for both devices, add another vCPU, claim cross-host/live migration compatibility, add external-storage durability semantics, or make performance/downtime claims.

The pair schema is a required nested transaction component, not a migration protocol by itself.

## Promotion rule

After the versioned registration-pair schema is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal the pair byte boundary rather than farming alternate envelope fields or corruption offsets.

The next architecture milestone is one **canonical two-device outer transaction** that binds `VersionedFullControllerTwoVirtioBlkCheckpointV1` and `VersionedHostRegistrationPairV1`. Its executable proof must consume both nested objects decoded from one byte stream, restore the two-device checkpoint exactly and reconstruct fresh acceleration from the decoded registration pair. An envelope that merely prints both nested metadata without using both decoded objects is not sufficient. Coordinated multi-vCPU ownership, cross-host/live migration, external-storage crash consistency and performance/downtime remain separate later frontiers.
