# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `ee289d59fb648b84db3e1042dbdaa526d6730827` through PR #106 (`Run a SIPI-started AP local LAPIC timer`). The repository integrates the Phase 73 foundation, x86-64/ELF64 execution, userspace and virtual MMIO, controller-backed interrupts, direct and irqfd/eventfd asynchronous delivery, PCI/virtio-rng/virtio-blk paths, and a bounded two-vCPU SMP control/data plane.

The SMP path now includes guest-owned INIT/SIPI AP startup, the AP real-mode-to-long-mode transition, guest-originated xAPIC IPIs, shared mailbox work dispatch, targeted MSI delivery, and one AP-owned local LAPIC one-shot timer. PR #106 proves that the SIPI-started AP programs and services vector `0x53` itself, performs EOI/IRETQ, publishes shared completion ownership, and preserves the integrated AP startup/long-mode state.

Exact merged-main ordinary CI and all permanent hosted-KVM workflows for `ee289d59fb648b84db3e1042dbdaa526d6730827` are green. The fixed AP-local-timer vector/count proof is sealed. Do not farm periodic/count/divide/vector variants merely to extend the phase number.

## Selected milestone — bounded cross-vCPU TLB shootdown

The next architecture boundary is shared address-space invalidation across vCPUs. Both vCPUs share CR3/page tables; the AP must first establish one alias translation, the BSP must then mutate that shared PTE and send a guest-originated xAPIC shootdown IPI, and the AP handler must execute `invlpg` before acknowledging completion. The AP must subsequently observe the replacement physical page through the same virtual address.

This is deliberately one fixed target page and one AP. It proves the end-to-end protocol and ownership ordering without claiming a general TLB-generation framework or relying on an architecturally unguaranteed stale read before the shootdown.

Acceptance contract:

- preserve ordinary CI, Rust 1.74 shipped-target MSRV, and every permanent hosted-KVM workflow already green on merged `main`;
- create exactly two vCPUs; vCPU1 must begin `KVM_MP_STATE_UNINITIALIZED` and start only through the existing guest BSP INIT assert/deassert plus SIPI vector `0x08` path;
- preserve the first 73 bytes of the integrated AP guest-owned real-mode-to-long-mode transition byte-for-byte;
- preserve shared CR3/page tables and the existing LAPIC mapping VA `0x500000` → GPA `0xfee00000`;
- fixed target alias VA `0x501000` uses PTE GPA `0x4808`, initially mapping RAM page A `0x18000` containing byte `A`;
- AP must read `A` through the target alias before reporting readiness, establishing the translation without making any stale-read timing claim;
- only after AP readiness may BSP guest code mutate PTE `0x4808` to page B `0x19000|0x3`, execute `mfence`, report barrier `P`, send xAPIC vector `0x54` to APIC ID1, and report `X`;
- AP owns IDT vector `0x54` with handler GPA `0x14000`; IDTR must be `0x6000/0x54f`;
- the handler must execute `invlpg [0x501000]` before observable byte `I`, publish exactly one shootdown acknowledgement, write LAPIC EOI, and `iretq`;
- after handler return the AP must read the same VA and require exact byte `B`, then report `B,D`;
- BSP may consume the acknowledgement only after the AP handler publishes it, then reports `A,D`;
- exact BSP proof is `0IDSPXAD`; exact AP proof is `ALRIBD`; every byte-wide debug-port exit must have exact direction, port, size, count and payload;
- final guest memory must retain page A=`A`, page B=`B`, final target PTE=`0x19003`, and consumed acknowledgement byte=`0`;
- AP ready state requires architectural RFLAGS bit1 with IF clear; AP completion requires bit1+IF;
- AP startup state must remain SIPI-compatible (`RIP=0`, CS selector `0x0800`, CS base `0x8000`, pre-transition CR0.PE clear), and post-shootdown state must retain stack `0x1ef000`, CS `0x08` with L=1, SS `0x10`, GDT `0x7000/0x17`, CR3 `0x1000`, required CR0/CR4/EFER bits, and the shootdown IDT;
- deterministic regressions must prove target PTE placement outside the LAPIC slot, preserve the integrated 73-byte SIPI prefix, and prove the handler's `invlpg` opcode precedes observable acknowledgement byte `I`;
- KVM-aware integration must independently validate startup/long-mode/IDT/RFLAGS state, both proof streams, every debug-port exit, final PTE, final acknowledgement, and both backing-page bytes;
- executable evidence must expose vector `0x54`, target VA `0x501000`, PTE `0x4808`, initial AP MP state `1`, exact startup/IDT state, final PTE `0x19003`, final ack `0`, page A `65`, page B `66`, both proof streams, and ready/completion RFLAGS;
- permanent workflow `Strict KVM two-vCPU TLB shootdown` must run independently on hosted KVM with a bounded timeout and require all executable evidence above; it may not skip `/dev/kvm` or convert a shootdown failure into success;
- formatter, Clippy, MSRV, startup, page-table mutation, IPI, `invlpg`, acknowledgement, proof, or architectural-state failures remain hard failures and must not be hidden by changed expectations, retries-to-success, or weakened permanent gates.

The implementation is in progress on `milestone/cross-vcpu-tlb-shootdown` / PR #107. Its initial compile/API integration blockers were corrected by aligning vCPU IDs with the repository's `u16` boundary and using the integrated MP-state reader; the existing ordinary and permanent workflows are green on that corrected foundation. Binary, KVM-aware integration and a dedicated permanent shootdown workflow are part of this same coherent slice. No capability is considered integrated until the final exact candidate passes all ordinary/permanent workflows, remains current with `main`, and completes the normal review/merge audit.

## Scope boundary

This milestone deliberately does **not** add:

- a global TLB generation counter, generic shootdown queue, multiple target pages, additional APs, broadcast invalidation, or arbitrary vCPU masks;
- PCID, INVPCID, CR3 broadcast/reload policy, large-page invalidation, nested paging control, or guest ASID management;
- a stale-read timing guarantee, TLB residency introspection, performance/latency benchmark, or timing-based correctness assertion;
- ring transition/TSS, SYSCALL/SYSRET, privilege separation, userspace/kernel address-space ownership, or per-process page tables;
- new timer variants, PCI/virtio/storage features, DMA/IOMMU, migration, persistence, or durability claims.

## Promotion rule

After the fixed one-target cross-vCPU shootdown is integrated and exact merged-`main` ordinary CI plus every permanent workflow are green, seal the proof rather than adding extra target addresses, vectors or AP clones.

The next architecture audit should select a genuinely higher-order boundary. Strong candidates are a bounded privileged execution transition with per-vCPU TSS/ring ownership, or a more general shared-address-space invalidation protocol only if it introduces real reusable generation/queue semantics plus executable multi-vCPU evidence. More fixed shootdown variants are not a promotion.
