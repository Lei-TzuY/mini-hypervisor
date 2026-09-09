# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `c7c438c735349f0294033f3830e05fd162fbdd04` through PR #126 (`Restore scheduler wait ownership from a bounded checkpoint`). Exact merged-main ordinary CI and every applicable permanent hosted-KVM workflow for that commit are settled successfully.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall dispatcher; fault-safe copyin/copyout/usercopy; two isolated ring3 address spaces; opt-in slot-0 dirty-page tracking; bounded guest-owned A→B→A task context switching; host-timer task preemption; one bounded Runnable→Blocked→Runnable lifecycle with external wakeup; a two-entry guest-owned runnable selector; one bounded wait-channel owner with wrong-channel rejection and correct-channel external wake ownership; incremental K/X/W/D dirty capture of the co-located scheduler/wait/task context page; one reusable full-VCPU plus one-page checkpoint; and scheduler wait ownership restored compositionally from that checkpoint at a non-serviceable Debug boundary.

PR #118 seals cooperative two-task context switching. PR #119 seals host-timer preemption. PR #120 seals external Blocked→Runnable wakeup. PR #121 seals the bounded `[A,B]` runnable selector. PR #122 seals the fixed one-owner/one-waiter wait-channel proof. PR #123 seals incremental K/X/W/D dirty capture of physical page `0x30000`. PR #124 keeps strict KVM execution timeouts independent of cold compilation. PR #125 seals the reusable one-VCPU/one-page checkpoint primitive. PR #126 seals scheduler wait ownership composition: capture at typed `Debug@0x1603d`, prove deliberate page/VCPU/wait/queue divergence, restore page and VCPU through the existing dependency order, perform fresh typed ownership verification, then resume unchanged `K1AXPW0BRD` execution on real KVM.

The one-owner/one-page scheduler composition is sealed. Do not farm alternate channel values, another fixed scheduler barrier or more one-page marker fixtures merely to extend the phase number.

## Selected milestone — bounded explicit multi-page VCPU checkpoint

The next architecture boundary is a coherent guest-memory ownership set rather than one arbitrary 4 KiB page. This milestone retains the existing `VcpuStateSnapshot` capture/restore dependency ordering and promotes only the memory side to an explicitly bounded, canonical page set. The executable proof must demonstrate that multiple pages carry independent roles and that resume depends on restoring all of them, rather than merely serializing several decorative pages.

The deterministic proof uses exactly three owned pages in one VM: control page `0x30000`, data page `0x31000`, and active stack page `0x1fe000`. Before checkpoint capture the guest writes `A` to control, writes `B` to data, pushes `C` onto the owned stack page and reaches HLT. Host code captures all three pages plus one full VCPU snapshot, corrupts all three pages with distinct full-page patterns and moves the VCPU to a different valid long-mode state, requires fresh mismatch evidence for every page and the VCPU, restores all owned pages and the VCPU, performs fresh exact verification, and only then resumes. The restored guest reads `A`, reads `B`, pops `C` from the restored stack and emits `R`, giving exact proof `ABCR`.

Acceptance contract:

- preserve exact merged-green base `c7c438c735349f0294033f3830e05fd162fbdd04`, Rust 1.74 shipped-target MSRV, ordinary CI, #125 one-page checkpoint behavior, #126 scheduler wait checkpoint composition and every applicable permanent hosted-KVM workflow;
- use one `BoundedVcpuPageSetCheckpoint` model containing 1..=8 explicit pages plus the existing `VcpuStateSnapshot`; do not create a second VCPU snapshot or restore ordering;
- reject empty ownership sets, more than eight pages, non-4KiB-aligned GPAs and duplicate pages before capture; canonicalize the accepted ownership set by GPA;
- deterministic owned pages are exactly `0x30000`, `0x31000` and `0x1fe000` and must be returned in canonical order;
- before capture the guest writes marker `A` to control, marker `B` to data and pushes marker `C` onto the actual stack page, then reaches non-serviceable HLT at RIP `0x10013`; the capture boundary must service no PIO/MMIO exit;
- deliberately corrupt each owned page with a distinct full-page pattern and change the VCPU to a different valid long-mode entry/stack state;
- fresh corruption verification must independently report control=false, data=false, stack=false and VCPU exact=false; no page mismatch may be inferred only from an aggregate boolean;
- restore must write every captured page, restore VCPU special registers/registers/MSRs through the already integrated dependency order, then perform fresh page and VCPU capture/comparison; guest resume is forbidden unless all three page comparisons and VCPU comparison are exact;
- the stack page is executable state, not decorative data: resumed code must `pop` marker `C` from restored RSP before proof can complete;
- exact resumed proof is bytes `[65, 66, 67, 82]` (`ABCR`) across four byte-wide debug-port exits and terminal HLT must be RIP `0x1002d` with architectural RFLAGS bit 1;
- KVM-aware integration must independently validate capture HLT, canonical ownership set, per-page corruption, per-page restore, VCPU mismatch→exact transition, all four debug exits, proof and terminal HLT;
- a permanent hosted-KVM workflow must build `multi-page-checkpoint` before an unchanged 30-second execution timeout and require exact ownership, mismatch/restore, `ABCR`, capture RIP and terminal RIP evidence; `/dev/kvm` absence is not a successful permanent-gate outcome;
- formatter, Clippy, MSRV, page-set validation, capture/restore ordering, per-page mismatch, VCPU mismatch, fresh exact verification, proof or hosted-KVM failures remain hard failures and must not be skipped, retried into success or hidden by changed expected values.

Implementation is in progress on `milestone/multi-page-checkpoint` through draft PR #127. The production page-set core and executable three-role fixture have already passed ordinary CI software checks and all existing strict KVM gates on an intermediate exact head. Only the final verification-synchronized exact candidate with the new dedicated multi-page hosted-KVM proof and every applicable workflow green may be integrated.

## Scope boundary

This milestone deliberately does **not** add:

- device/controller, irqchip/LAPIC/PIC, irqfd/eventfd/ioeventfd, PCI or virtio state checkpoint/restore;
- automatic dirty-page ownership selection, dirty-ring/manual-protect2, pre-copy/post-copy policy or replay logs;
- capture or replay from arbitrary in-flight PIO/MMIO service exits;
- an unbounded page list, multi-slot memory image or full guest-RAM snapshot;
- multi-VCPU coordinated snapshot, SMP quiescence or task migration checkpointing;
- FPU/XSAVE/debug-register migration claims beyond the already integrated VCPU snapshot contract;
- migration serialization/versioning, cross-host compatibility or crash-consistent whole-VM snapshots;
- demand paging, copy-on-write, swapping, DMA/IOMMU or performance/latency claims.

## Promotion rule

After the bounded explicit multi-page checkpoint is integrated and exact merged-`main` ordinary CI plus every applicable permanent hosted-KVM workflow are green, seal this three-role page-set proof rather than adding fourth/fifth arbitrary pages or alternate marker values.

The next architecture audit should prefer device/controller-state checkpoint composition only when capture/restore ordering, external kernel-owned state, quiescence boundaries and executable KVM evidence can be made explicit. Another valid higher frontier is coordinated multi-VCPU checkpoint/quiescence if ownership and stop-the-world semantics can be proven without weakening existing SMP invariants. Migration serialization/versioning, in-flight service-exit replay and cross-host portability remain separate higher-order frontiers.
