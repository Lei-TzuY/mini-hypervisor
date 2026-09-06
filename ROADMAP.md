# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 execution, ELF64 loading/mapping, userspace and virtual MMIO, controller-backed interrupts, async timer delivery through direct GSI and irqfd/eventfd, ioeventfd signaling, guest-discovered PCI BAR MMIO, bounded virtio-rng/virtio-blk split-ring execution with INTx/MSI completion, direct and indirect block descriptors, bounded multi-sector and write/readback storage behavior, and a two-vCPU SMP control plane with targeted MSI, guest-originated fixed IPI, and guest-driven INIT/SIPI application-processor startup.

Current `main` is `c8fecc94a0ab8270946dfdcc251168c01f7ee683` through PR #101. The INIT/SIPI fixture starts vCPU1 as `KVM_MP_STATE_UNINITIALIZED`, has BSP vCPU0 issue xAPIC INIT assert/deassert plus SIPI vector `0x08`, consumes Linux KVM's one startup `EAGAIN` handoff, and then proves the AP begins at real-mode `CS=0x0800`, base `0x8000`, `RIP=0`, `CR0.PE=0`. Its AP proof is `APD`; the BSP proof is `0IDSMD`; the AP writes marker `K` to shared guest RAM. Exact candidate CI #602 and the permanent INIT/SIPI workflow were green, and merged-main permanent workflows are green at this boundary.

The fixed APIC-ID1/vector-0x08 real-mode startup phase is sealed. Do not farm alternate SIPI vectors, repeated reset/startup cycles, vCPU2/vCPU3 clones, or host-side shortcuts merely to extend the phase number. Independent storage durability remains a separate frontier.

## Selected milestone — guest-driven AP real-mode to long-mode transition

The next SMP boundary is execution-mode ownership. The integrated AP currently proves only SIPI-selected real-mode execution. This milestone must reuse the exact guest-driven INIT/SIPI lifecycle but have the AP trampoline itself join the existing 64-bit execution environment. Userspace must not call `initialize_long_mode()` on vCPU1 after SIPI or directly rewrite AP control/segment registers into the desired final state.

The BSP already installs the bounded low-2MiB identity page tables at PML4 `0x1000`, PDPT `0x2000`, PD `0x3000`, with the existing LAPIC alias mapping. The AP may reuse those guest-resident page tables, but the AP guest code owns the transition: install a bounded guest GDT, set CR4.PAE, load CR3, set EFER.LME, set CR0.PE|PG, far-jump through a 64-bit code descriptor, normalize data/stack segments, select an AP-only stack, then produce its 64-bit proof and shared-memory handoff.

Acceptance contract:

- preserve all existing main CI and permanent workflow contracts, including the historical `Strict KVM two-vCPU INIT SIPI` proof and Rust 1.74 MSRV;
- preserve `run_two_vcpu_init_sipi()` and its exact real-mode AP proof `APD`; the new path must share the same BSP/INIT/SIPI/startup-checkpoint orchestration rather than duplicate a second startup flow;
- vCPU1 must still begin `KVM_MP_STATE_UNINITIALIZED`, consume exactly the established KVM startup `EAGAIN` handoff, and prove startup `MP_STATE=RUNNABLE`, `RIP=0`, `CS.selector=0x0800`, `CS.base=0x8000`, `CR0.PE=0` before any AP trampoline instruction executes;
- userspace must not call `initialize_long_mode()` on vCPU1 or set AP CR0/CR3/CR4/EFER/CS to manufacture the result;
- the long-mode AP trampoline starts at GPA `0x8000`, keeps IF clear, emits `A` while still in SIPI real mode, then performs `LGDT`, CR4.PAE enable, `CR3=0x1000`, `EFER.LME`, `CR0.PE|PG`, and a far jump through selector `0x08` into 64-bit code;
- install a bounded AP GDT at GPA `0x7000` with null, 64-bit code selector `0x08`, and data selector `0x10`; GDTR is at `0x7020`, limit `0x17`; these addresses must not overlap bootstrap page tables, AP trampoline, marker or BSP/AP stacks;
- after the far jump, normalize SS/DS/ES to selector `0x10` and set an AP-only stack `RSP=0x1ef000`, distinct from the BSP stack `0x1ff000`;
- the AP's exact proof is `ALPD`: `A` proves the SIPI real-mode entry, `L` is emitted only after the far jump into 64-bit code and AP stack installation, `P` follows the 64-bit write of marker `K` to GPA `0x9000`, and `D` is the AP completion barrier;
- the BSP proof remains exactly `0IDSMD` and may emit its final `M` only after shared guest RAM contains marker `K` written by the AP long-mode path;
- final AP `KVM_MP_STATE` remains RUNNABLE; RFLAGS architectural bit1 is set and IF remains clear;
- final architectural state must require `RSP=0x1ef000`, `CS.selector=0x08`, `CS.base=0`, `CS.L=1`, `CS.DB=0`, `SS.selector=0x10`, GDT base `0x7000`/limit `0x17`, `CR0` containing PE|PG, `CR4` containing PAE, `CR3=0x1000`, and `EFER` containing LME|LMA;
- KVM-aware integration independently validates both proof streams, startup checkpoint, marker, MP/RFLAGS state, all AP long-mode control/segment/table fields, and every debug-port exit;
- add a permanent `Strict KVM two-vCPU AP long mode` workflow that executes the new binary on hosted KVM and requires all of the above state evidence; retain the existing permanent INIT/SIPI workflow unchanged as the historical lower-layer proof;
- generated assembler/linker artifacts or construction scripts must not be committed; the deterministic machine-code bytes and GDT/GDTR layout are checked by regression tests;
- any INIT/SIPI, EAGAIN handoff, instruction encoding, GDT, control-register, far-jump, long-mode state, shared-marker, proof, MSRV or real-KVM failure remains a hard failure and must not be skipped, slept into success, retried into success, or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** add:

- per-CPU IDT/TSS, privilege transitions, SYSCALL/SYSRET, userspace/kernel rings, or a general descriptor-table allocator;
- x2APIC, logical destination mode, additional APs, ACPI/MADT discovery, firmware AP tables, hotplug or repeated INIT/SIPI cycles;
- a general SMP scheduler, shared runnable queue, cross-vCPU locking framework, atomics/futexes or memory-model claims;
- additional virtio/storage functionality, persistent flush/durability, DMA/IOMMU or migration;
- performance, latency, scalability or benchmark claims.

## Promotion rule

After the AP guest-driven long-mode transition is integrated and exact merged-`main` permanent workflows are green, seal the fixed two-vCPU startup/mode-transition proof rather than adding alternate GDT addresses, stack values or more SIPI vectors.

The next SMP architecture audit should choose a materially higher execution boundary: for example an AP-owned 64-bit interrupt/IDT path, a bounded two-vCPU shared-memory synchronization primitive with executable ordering evidence, or another cross-vCPU workload that requires both CPUs to participate in the 64-bit environment. Prefer the smallest coherent slice that adds a new interaction model rather than cloning the startup fixture. Persistent backing/durability remains an independent storage frontier.
