# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 execution, ELF64 loading/mapping, userspace and virtual MMIO, controller-backed interrupts, async timer delivery through direct GSI and irqfd/eventfd, ioeventfd signaling, guest-discovered PCI BAR MMIO, bounded virtio-rng/virtio-blk execution, and the two-vCPU SMP control plane through guest-driven INIT/SIPI, AP-owned real-mode-to-long-mode transition, and guest-originated xAPIC IPI delivery into the running long-mode AP.

Current `main` is `bfc8200c256ed2218cd9b1110d0ec71d6f194d53` through PR #103. The AP begins `KVM_MP_STATE_UNINITIALIZED`, is started by BSP guest xAPIC INIT assert/deassert plus SIPI vector `0x08`, performs its own transition to 64-bit mode, owns a bounded IDT gate, reports readiness with IF clear, receives guest-originated vector `0x52`, executes its handler with LAPIC EOI and `iretq`, resumes mainline, and completes with IF set. Main CI #625 and the applicable permanent hosted-KVM workflows are green at this boundary.

The fixed AP-startup/vector-`0x52` control-plane phase is sealed. Do not farm alternate SIPI vectors, APIC IDs, IDT addresses, repeated INIT/SIPI cycles, or vCPU2/vCPU3 clones merely to extend the phase number.

## Selected milestone — bounded concurrent two-vCPU shared-memory work dispatch

The next data-plane boundary is one executable shared-memory synchronization/work-dispatch protocol that requires both running vCPUs to make progress. This slice deliberately extends the existing directly initialized two-long-mode-vCPU data-plane fixture rather than claiming to be the SIPI-started AP from PRs #101–#103. The startup/control-plane proofs remain independent and must not regress.

BSP and vCPU1 run concurrently on distinct host threads in one VM and coordinate through a guest-RAM mailbox at GPA `0x9000`. The concrete protocol uses implicitly locked byte `XCHG` operations for command and acknowledgement ownership. This is evidence for this exact x86 mailbox handoff only, not a formal C/C++/Rust memory model, generic atomics library, scheduler, or scalable work queue.

Acceptance contract:

- preserve ordinary CI, Rust 1.74 MSRV, all existing strict KVM gates, and the permanent INIT/SIPI/AP-long-mode/AP-IPI workflows;
- retain one VM with exactly two directly initialized long-mode vCPUs for this isolated data-plane fixture; do not describe vCPU1 as the SIPI-started AP;
- run BSP and vCPU1 concurrently on distinct host threads after an explicit vCPU1 readiness barrier;
- mailbox GPA is `0x9000`; payload is byte `0x21` at offset `0x00`, command at `0x08`, result at `0x10`, and acknowledgement at `0x18`;
- BSP stores payload before publishing command `1` with memory `XCHG`; vCPU1 uses a bounded poll loop, claims command with memory `XCHG`, doubles payload to result `0x42`, then publishes acknowledgement `1` with memory `XCHG`;
- BSP uses a bounded poll loop, consumes acknowledgement with memory `XCHG`, then validates result; command and ack must both finish cleared to zero;
- bounded poll exhaustion or ownership/value mismatch emits byte `F` and halts instead of spinning indefinitely or being retried into success;
- exact BSP proof is `BCVD`; exact vCPU1 proof is `RPD`; every byte-wide debug-port exit must match direction, size, port and payload;
- exact final mailbox state is payload `0x21`, command `0`, result `0x42`, ack `0`;
- exact terminal reports are BSP `KVM_EXIT_HLT` at RIP `0x10043` and vCPU1 `KVM_EXIT_HLT` at RIP `0x1103c`, with architectural RFLAGS bit1 set on both;
- KVM-aware integration independently validates both proof streams, every debug-port exit, final mailbox state and both terminal reports;
- permanent workflow `Strict KVM two-vCPU work dispatch` must run independently from ordinary CI and require the exact proofs, mailbox values and terminal RIP/RFLAGS contract;
- generated assembler/linker artifacts, temporary construction scripts and alternative duplicate runtime paths are not committed;
- mailbox ownership, proof, terminal-state, MSRV or real-KVM failures remain hard failures and must not be skipped, retried into success, or hidden by changed expectations.

Executable evidence has been established before this roadmap synchronization on implementation head `259ecdbef5a3420549650a647f7c6f0e6bff348d`. Ordinary CI #628 passed Format, Clippy, full tests, build, rustdoc, Rust 1.74 MSRV and all current standard strict KVM gates. Permanent `Strict KVM two-vCPU work dispatch` run #3 completed successfully on hosted KVM and produced BSP proof `BCVD`, vCPU1 proof `RPD`, mailbox `0x21/0/0x42/0`, BSP HLT RIP `0x10043` with RFLAGS `0x46`, and vCPU1 HLT RIP `0x1103c` with RFLAGS `0x6`. Because this roadmap commit changes the candidate head, the final exact candidate must rerun all applicable CI/permanent workflows before integration.

## Scope boundary

This milestone deliberately does **not** add:

- a general scheduler, job queue, multi-producer/multi-consumer protocol, futex, lock library or formal language-level memory model;
- a third vCPU, AP hotplug, repeated INIT/SIPI, new fixed IPI vectors or a replacement AP-startup flow;
- AP-local timer ownership, periodic scheduling, cross-vCPU TLB shootdown, per-CPU TSS, ring transitions or SYSCALL/SYSRET;
- additional virtio/storage behavior, persistence/durability, DMA/IOMMU or migration;
- performance, latency, fairness, scalability or benchmark claims.

## Promotion rule

After the bounded work-dispatch primitive is integrated and exact merged-`main` ordinary CI plus the new permanent work-dispatch workflow are green, seal the one-item `0x21→0x42` mailbox proof rather than adding a second payload or another polling constant.

The preferred next SMP promotion is a cross-layer composition that carries this locked-`XCHG` work protocol through the already integrated SIPI-started, guest-owned long-mode AP lifecycle, ideally using the established guest-originated IPI control plane to signal work readiness. That slice should retire the current separation between the isolated data-plane fixture and the AP-startup/control-plane fixture rather than create a third startup path. If that composition is not the highest-value feasible step after audit, choose another workload that genuinely requires both processors to make progress. Persistent storage durability remains a separate frontier.
