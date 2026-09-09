# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `4d14b4d26b0d8faa1290a314412d811f95958c2d` through PR #128 (`Keep SYSCALL execution timeout independent of cold build`). Exact ordinary CI and every applicable permanent hosted-KVM workflow for that commit are settled successfully.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; two isolated ring3 address spaces; opt-in slot-0 dirty-page tracking; bounded guest-owned task switching and timer preemption; blocked/runnable/wait-channel ownership; scheduler/wait checkpoint composition; one reusable full-VCPU plus one-page checkpoint; and one bounded explicit multi-page VCPU checkpoint.

PR #127 seals the multi-page checkpoint frontier. Its deterministic real-KVM proof captures exactly three independent page roles — control `0x30000`, data `0x31000`, active stack `0x1fe000` — plus one full `VcpuStateSnapshot`; deliberately corrupts every page and the VCPU; proves each mismatch independently; restores pages plus VCPU through the established dependency order; then resumes only after fresh exact verification. Exact proof is `ABCR`, with capture HLT at RIP `0x10013` and terminal HLT at `0x1002d`. PR #128 changes no guest semantics: it builds the SYSCALL proof before the unchanged 30-second execution timeout so cold compilation cannot masquerade as KVM execution failure.

The one-VCPU multi-page checkpoint phase is sealed. Do not farm fourth/fifth arbitrary pages, alternate marker bytes or another fixed single-VCPU checkpoint fixture merely to extend the phase number.

## Selected milestone — bounded coordinated two-VCPU checkpoint

The next architecture boundary is stop-the-world ownership across more than one VCPU. Existing state machinery already has canonical per-VCPU register/special-register/MSR capture/restore and a bounded canonical page-set checkpoint; existing SMP fixtures already create two VCPU objects in one VM. This milestone composes those capabilities without cloning page capture logic: both VCPUs must first reach explicit non-serviceable HLT quiescent boundaries, then one shared page-set and exactly two canonical VCPU snapshots become one coordinated checkpoint.

The deterministic proof uses VCPU ids 0 and 1. VCPU0 starts at `0x10000`, writes shared marker `S` to page `0x30000`, pushes private marker `0` onto active stack page `0x1fd000`, and reaches capture HLT at RIP `0x1000b`. VCPU1 starts at `0x11000`, pushes private marker `1` onto active stack page `0x1fc000`, and reaches capture HLT at RIP `0x11003`. Only after both HLT boundaries have been observed may the checkpoint capture pages `[0x30000,0x1fc000,0x1fd000]` plus both VCPU snapshots.

Acceptance contract:

- preserve exact merged-green base `4d14b4d26b0d8faa1290a314412d811f95958c2d`, Rust 1.74 shipped-target MSRV, ordinary CI and every applicable permanent hosted-KVM workflow;
- reuse `BoundedVcpuPageSetCheckpoint` as the single owner of shared page capture/restore; do not duplicate the memory-copy pipeline for the second VCPU;
- accept exactly two distinct VCPU ids, canonicalize them by id and bind restore/verification to those exact ids; duplicate or mismatched VCPU ownership is a hard failure;
- require both VCPUs to reach their explicit non-serviceable HLT capture boundaries with no serviced PIO before coordinated capture begins;
- deterministic ownership pages are exactly shared page `0x30000`, VCPU1 stack page `0x1fc000` and VCPU0 stack page `0x1fd000`, returned in canonical GPA order;
- captured roles must independently contain shared marker `S`, VCPU0 private stack marker `0` at `0x1fdff0`, and VCPU1 private stack marker `1` at `0x1fcff0`;
- deliberately corrupt all three owned pages with distinct full-page patterns and move both VCPUs to different valid long-mode entry/stack states;
- fresh corruption verification must independently report all three pages non-exact and both VCPU snapshots non-exact; aggregate-only mismatch evidence is insufficient;
- restore the shared page-set exactly once through the existing page-set checkpoint, restore the lower-id VCPU through that checkpoint, then restore the second VCPU snapshot; only fresh exact verification of every page and both VCPUs permits resume;
- resumed VCPU0 must pop restored marker `0`, verify restored shared `S`, emit exact byte-wide debug proof `0` and halt at RIP `0x10020`; resumed VCPU1 must independently pop restored marker `1`, verify restored shared `S`, emit proof `1` and halt at RIP `0x11018`; either guest has an explicit `F` failure path if restored execution state is wrong;
- KVM-aware integration must independently validate both capture reports, canonical ownership set, per-page mismatch→exact transition, per-VCPU mismatch→exact transition, both PIO exits/proofs and both terminal HLT reports;
- a permanent hosted-KVM workflow must build `two-vcpu-checkpoint` before an unchanged 30-second execution timeout and require exact capture RIPs, ownership set, corruption/restore booleans, proofs `[48]` and `[49]`, terminal RIPs and architectural RFLAGS bit 1; `/dev/kvm` absence is not a successful permanent-gate outcome;
- formatter, Clippy, MSRV, quiescence, ownership binding, capture/restore ordering, page/VCPU mismatch, fresh verification, proof or hosted-KVM failures remain hard failures and must not be skipped, retried into success or hidden by changed expected values.

Implementation is in progress on `milestone/two-vcpu-checkpoint` through draft PR #129. The coordinated checkpoint core has passed ordinary CI software checks and all existing strict KVM steps on an intermediate exact head. Only the final verification-synchronized candidate with the dedicated two-VCPU hosted-KVM proof and every applicable workflow green may be integrated.

## Scope boundary

This milestone deliberately does **not** add:

- device/controller, irqchip/LAPIC/PIC, irqfd/eventfd/ioeventfd, PCI or virtio state checkpoint/restore;
- capture or replay from arbitrary in-flight PIO/MMIO service exits;
- concurrent host-thread VCPU execution, a general stop-the-world scheduler, INIT/SIPI orchestration or cross-VCPU interrupt checkpointing;
- more than two VCPUs, an unbounded page list, multi-slot memory image or full guest-RAM snapshot;
- automatic dirty-page ownership selection, dirty-ring/manual-protect2, pre-copy/post-copy or replay logs;
- FPU/XSAVE/debug-register migration claims beyond the already integrated VCPU snapshot contract;
- migration serialization/versioning, cross-host compatibility or crash-consistent whole-VM snapshots;
- DMA/IOMMU state, demand paging, copy-on-write, swapping or performance/latency claims.

## Promotion rule

After the bounded coordinated two-VCPU checkpoint is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this fixed two-VCPU proof rather than adding VCPU2/VCPU3 clones or alternate marker values.

The next architecture audit should revisit device/controller-state checkpoint composition now that bounded memory ownership and multi-VCPU quiescence/restore ordering are both executable. That promotion is valid only when kernel-owned PIC/LAPIC/IOAPIC/irqfd state, capture/restore ordering and quiescence boundaries can be represented explicitly and proven on real KVM. Migration serialization/versioning, in-flight service-exit replay and cross-host portability remain separate higher-order frontiers.
