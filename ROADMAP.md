# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `a51e72e40ac7913e83a1237107e7b13403989d64` through PR #146 (`Version two-device full-controller checkpoints`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe usercopy; isolated ring3 address spaces; dirty-page tracking; bounded guest scheduling/wait ownership; bounded page/VCPU/controller/device checkpoints; fd-free ioeventfd/irqfd reconstruction around restore; canonical versioned byte schemas for page+VCPU, full-controller, full-controller+one-quiescent-virtio-blk, host-registration reconstruction and full-controller+exactly-two-quiescent-virtio-blk; and one canonical one-device outer checkpoint transaction.

PR #146 sealed the fixed-two-device byte compatibility boundary. Exact merged-main commit `a51e72e40ac7913e83a1237107e7b13403989d64` completed all 46 push-triggered workflows successfully, including ordinary `CI`, MSRV validation, the strict two-device runtime checkpoint proof and the strict versioned two-device checkpoint proof. Do not farm additional device counts, alternate framing, BAR permutations or corruption offsets merely to extend this phase.

## Selected milestone — two-device reconstructed acceleration

The next executable boundary is host acceleration ownership for the already-sealed two-device checkpoint. The codebase can reconstruct one fd-free ioeventfd/irqfd tuple and can route two independent legacy-PIC MMIO interrupt sources, but it has not yet proven that exactly two semantic registration descriptors can be owned, reconstructed, used and cleaned up as one bounded pair across two independent GSI/vector paths.

Implementation continues on `milestone/two-host-registration-acceleration` from exact green `main=a51e72e40ac7913e83a1237107e7b13403989d64`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow green on the exact base;
- define one fixed pair ownership type over exactly two existing fd-free `HostRegistrationSpec` descriptors without serializing or retaining raw file descriptors;
- canonicalize the pair by ascending doorbell address and reject overlapping doorbell ranges or duplicate GSI ownership before any kernel registration is created;
- use deterministic virtio notification doorbells `0x10000100` and `0x10001100`, both two bytes wide with datamatch zero, routed independently to GSI 0/vector `0x40` and GSI 1/vector `0x41`;
- reconstruct the first tuple and then the second; if second reconstruction fails, the first tuple must be deassigned before returning failure;
- pair deassignment must attempt both members in reverse registration order even when one cleanup operation fails, so a cleanup error cannot silently strand the other registration;
- prove the accelerated doorbell writes are consumed by KVM_IOEVENTFD rather than a userspace MMIO fallback: after each guest doorbell write the next expected exit is a debug-port barrier, and an unexpected KVM MMIO exit remains a hard failure;
- prove one ioeventfd event and one irqfd signal for each device independently, with two distinct interrupt handlers and legacy-PIC EOIs;
- prove deterministic cleanup and fresh reconstruction by executing both device paths in one first registration generation, deassigning the entire pair, reconstructing the same semantic pair into fresh process-local kernel resources, then executing both device paths again;
- the real-KVM proof must retain exact ordered output `RA0MB1NCE0PF1QD`, doorbell event counts `[[1, 1], [1, 1]]`, canonical doorbell/GSI/vector identities, and completion RFLAGS bit 1 plus IF;
- deterministic unit coverage must prove canonical pair ordering, non-overlapping ownership, duplicate-GSI rejection and fd-free checkpoint ownership;
- add an independent KVM integration test, proof binary and permanent hosted-KVM workflow with the binary build outside the 30-second execution timeout;
- registration, rollback, cleanup, routing, ioeventfd, irqfd, interrupt ordering or proof failures remain hard failures and must not be swallowed, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** define a multi-registration byte schema, widen the existing outer checkpoint transaction, serialize raw eventfd/ioeventfd/irqfd descriptors, process actual virtio-blk queue contents, add more than two registration descriptors, add another vCPU, claim cross-host/live migration compatibility, add external-storage durability semantics, or make performance/downtime claims.

The two doorbells intentionally correspond to the two sealed virtio-blk BARs, but this slice proves host-acceleration ownership and interrupt delivery rather than a second full block-request data path.

## Promotion rule

After two-device reconstructed acceleration is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this pair lifecycle rather than farming more GSI values, generations or doorbell aliases.

The next architecture frontier is a **canonical two-registration specification and outer transaction**: serialize the sealed semantic registration pair, bind it with the already-versioned two-device checkpoint, discard the process-local semantic objects, decode/materialize only from one canonical byte stream, reconstruct fresh acceleration and prove exact checkpoint restore plus accelerated replay. Coordinated multi-vCPU ownership, cross-host/live migration, external-storage crash consistency and performance/downtime remain separate later frontiers.
