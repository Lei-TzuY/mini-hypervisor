# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` is `c09f6a17b5c70a1c6c796d948a45e49c720b3b13` through PR #115 (`Expose bounded partial-progress copy through syscall dispatch`). Exact merged-main ordinary CI and every triggered permanent hosted-KVM workflow completed successfully; no failed, queued, in-progress or cancelled workflow remains for that exact main commit.

The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; guest-owned ring3/TSS transitions; a bounded SYSCALL/SYSRET ABI and syscall-number dispatcher; fault-safe copyin/copyout/usercopy; bounded four-byte cross-page partial-progress usercopy; and syscall nr2 exposing the same exact partial-progress semantics to ring3 callers while retaining nr0/nr1 and `-ENOSYS` compatibility.

PR #115 seals the fixed-length partial-copy syscall composition. Do not farm nr3/nr4 clones, longer hard-coded copies, duplicate fault sites or larger static fixup tables merely to extend that phase.

## Selected milestone — bounded guest-owned address-space switch

The next boundary is address-space ownership rather than another syscall or usercopy variant. One vCPU will execute two ring3 contexts that use the same user virtual code/data/stack addresses but different physical backing pages and different CR3 roots. A shared supervisor-only CPL0 handler switches A→B→A by changing CR3 and constructing complete user `iretq` frames; a terminal handler proves the original A mapping survives the round trip.

Acceptance contract:

- preserve exact merged-green base `c09f6a17b5c70a1c6c796d948a45e49c720b3b13`, Rust 1.74 shipped-target MSRV, ordinary CI and every permanent hosted-KVM workflow green on PR #115 merged main;
- retain address-space A at the integrated privilege root `CR3=0x1000`; create a second validated four-level low-2MiB root with PML4 `0xb000`, PDPT `0xc000`, PD `0xd000` and PT `0xe000`;
- both roots use the same user virtual entry `0x11000`, data page `0xa000` and stack page `0x1fc000`, but root B maps them to physical `0x20000`, `0x21000` and `0x22000` respectively;
- all shared kernel code, switch/terminal handlers, GDT, IDT, TSS, page tables and kernel stack remain identity-mapped supervisor-only in both roots;
- root A user code writes `A` to VA `0xa000`, invokes vector `0x80`, and later resumes exactly at `0x1100f` before invoking terminal vector `0x81`;
- the vector `0x80` CPL0 handler must read and record the active CR3, read the current address-space byte at the same VA `0xa000`, then switch A→B or B→A with `mov cr3` and return through a complete SS/RSP/RFLAGS/CS/RIP `iretq` frame;
- root B user code at the same VA writes `B` to its independently backed data page and invokes the same vector `0x80`; no host-side CR3 rewrite substitutes for guest-owned switching;
- CR3 observations must be exactly `[0x1000, 0xb000, 0x1000]`, final vCPU CR3 must be `0x1000`, physical A data must be `A` and physical B data must be `B`;
- exact debug proof is `ABAD`: first handler sees A data, second handler sees B data, terminal handler after the switch-back sees A data again, then emits `D` before halting;
- root-A and root-B user code/data/stack PTEs must map the expected distinct physical frames with P/W/U set; the shared switch-handler PTE must map the same identity physical page in both roots with U clear;
- the terminal path must halt in CPL0 at RIP `0x13031` with architectural RFLAGS bit 1 set; unexpected CR3 values fail closed with `F` and halt;
- KVM-aware integration must independently validate proof bytes, all four debug-port exits, CR3 observations/final CR3, physical data isolation, both roots' PTE ownership and terminal state;
- a permanent `address-space-switch` hosted-KVM workflow must run the standalone binary under a bounded timeout with mandatory `/dev/kvm` and hard-check the same CR3/data/PTE/proof/terminal invariants;
- formatter, Clippy, Rust 1.74 MSRV, page-table construction, U/S ownership, CR3 transitions, user return frame, proof or hosted-KVM failures remain hard failures and must not be hidden by changed expectations.

Implementation is in progress on `milestone/address-space-switch`. The implementation must pass ordinary CI and the new permanent hosted-KVM proof on one exact candidate before integration.

## Scope boundary

This milestone deliberately does **not** add:

- a general task/process object model, scheduler, runqueue, timer preemption or context-switch policy;
- more than two fixed user address spaces, dynamic address-space allocation, arbitrary virtual mappings, fork, mmap, copy-on-write, demand paging or swapping;
- PCID/ASID optimization, KPTI, SMEP, SMAP, PKU or security claims beyond the executed U/S and physical-isolation invariants;
- multi-vCPU user execution, cross-vCPU process migration, per-process TSS ownership, signals or resumable task snapshots;
- new filesystem/device/syscall surfaces, PCI/virtio capability, DMA/IOMMU or performance claims.

## Promotion rule

After the two-address-space CR3 switch is integrated and exact merged-`main` ordinary CI plus every triggered permanent hosted-KVM workflow are green, seal the fixed A/B proof rather than adding a third hard-coded address space or more switch vectors.

The next architecture audit should promote to bounded task context ownership only if a genuine executable slice can combine address-space identity with saved user register/stack state and a real scheduling/dispatch transition. Otherwise choose another materially different caller-visible or runtime-control boundary; do not promote by cloning address spaces or enlarging static fixtures.
