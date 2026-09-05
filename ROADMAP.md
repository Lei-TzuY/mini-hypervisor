# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 loading/mapping, userspace and virtual MMIO, direct/controller-backed interrupt delivery, MMIO interrupt lifecycles and multi-device routing, host-driven timer delivery through direct `KVM_IRQ_LINE` and irqfd/eventfd, ioeventfd-backed device signaling, guest-discovered PCI BAR MMIO, bounded virtio-rng split-ring execution with INTx/MSI completion, bounded virtio-blk read/INTx execution, same-VM in-memory virtio-blk `T_OUT`→`T_IN` write/readback, bounded four-sector multi-sector read/write, negotiated virtio-blk indirect descriptor execution, the bounded two-vCPU same-VM shared-memory handoff, and host-targeted fixed MSI delivery to a uniquely thread-owned second vCPU.

Current `main` is `bf9a2933325720cec0bc8cadf437191d60d4739d` through PR #99. PR #98 integrated negotiated `VIRTIO_RING_F_INDIRECT_DESC` execution while preserving the direct descriptor path. PR #99 then promoted the SMP surface independently of storage: vCPU1 is explicitly read back as `KVM_MP_STATE_RUNNABLE`, ownership moves to one worker thread, and host `KVM_SIGNAL_MSI` targets physical APIC ID 1 with vector `0x51`. Its exact proofs are `0MD` on vCPU0 and `RI1D` on vCPU1, with one MSI delivery and architectural RFLAGS/MP-state validation. The permanent hosted-KVM workflows for CI, virtio-blk INTx/write-readback/multi-sector/indirect execution, the two-vCPU foundation and targeted MSI are green on the integrated boundary.

The negotiated indirect-descriptor phase and the fixed host-targeted-MSI phase are sealed. Do not farm larger fixed indirect tables, extra payload signatures, more fixed MSI vectors, additional destination APIC IDs or vCPU2/vCPU3 clones merely to extend the phase number. The independently named persistent-flush storage surface remains a separate storage frontier and is not part of the current SMP slice.

## Selected milestone — guest-originated xAPIC fixed IPI to the thread-owned second vCPU

The next multiprocessor boundary is guest-visible interrupt control. The integrated two-vCPU proof can already target vCPU1 from the host through `KVM_SIGNAL_MSI`; this milestone requires vCPU0 itself to program the in-kernel legacy xAPIC ICR and deliver one fixed IPI to APIC ID 1 while preserving the existing unique vCPU1 worker ownership and explicit RUNNABLE startup.

This is a deliberately bounded xAPIC IPI proof, not guest AP startup or a general APIC model. vCPU1 remains host-promoted to RUNNABLE; INIT/SIPI startup is reserved for a later architectural phase.

Acceptance contract:

- preserve current main CI, every permanent virtio-blk workflow, the two-vCPU foundation and host-targeted-MSI workflow, all existing long-mode/ELF64/MMIO/interrupt/PCI/virtio contracts, and Rust 1.74 MSRV;
- do not modify or compete with the independently named persistent-flush storage surface;
- keep vCPU1 uniquely owned by one worker thread after setup; no `Vcpu: Sync`, shared concurrent vCPU access or scheduler abstraction is introduced;
- retain explicit `KVM_MP_STATE_RUNNABLE` promotion/readback for vCPU1 so this milestone isolates IPI delivery from INIT/SIPI startup;
- reuse the bounded virtual-MMIO page-table machinery to map guest VA `0x500000` to local-APIC GPA `0xFEE00000` without weakening RAM-backed mapping validation;
- install fixed vector `0x52` on both vCPUs so wrong-target self-delivery is observable rather than silently ignored;
- vCPU0 must execute xAPIC ICR high write `0x01000000` at offset `0x310` followed by ICR low fixed-delivery value `0x52` at offset `0x300`; userspace must not emulate these LAPIC accesses;
- if either ICR write or the handler EOI escapes the in-kernel LAPIC as `KVM_EXIT_MMIO`, execution fails rather than converting the milestone into userspace APIC emulation;
- vCPU1 emits readiness `R` under CLI, then remains blocked on the host synchronization channel until vCPU0 has crossed the post-ICR `S` barrier;
- after authorization, vCPU1 executes `sti; nop` and must observe handler `I` before mainline `1`, then `D`, producing exact proof `RI1D`;
- the handler must acknowledge the local APIC by writing zero to EOI offset `0xB0` through the same LAPIC alias before `iretq`;
- vCPU0 exact proof is `0SMD`; any handler `I` on vCPU0 before `S`, `M` or `D` hard-fails destination isolation;
- exact metadata is LAPIC alias `0x500000`, LAPIC GPA `0xFEE00000`, destination APIC ID `1`, vector `0x52`, ICR high `0x01000000`, ICR low `0x52`, and second MP-state `0` (`RUNNABLE`);
- RFLAGS architectural bit 1 remains set; IF is enabled at all vCPU0 barriers/completion, clear at vCPU1 readiness, and enabled at vCPU1 completion;
- KVM-aware integration independently validates exact LAPIC/ICR metadata, MP-state, both proof streams and all byte-wide debug-port exits;
- a permanent `Strict KVM two-vCPU guest IPI` workflow must independently require the same contract on hosted `/dev/kvm`, while the existing targeted-MSI workflow remains green as a separate host-originated transport proof;
- mapping, ICR encoding, destination isolation, LAPIC EOI, MP-state, worker ownership, proof order, RFLAGS, MSRV or real-KVM failures remain hard failures and must not be swallowed, skipped into success, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** add:

- guest INIT/SIPI application-processor startup, SIPI trampoline/real-mode bootstrap or AP reset sequencing;
- x2APIC, logical destination mode, broadcast/shorthand destinations, lowest-priority, NMI, SMI or INIT delivery modes;
- LAPIC timer/TSC-deadline, interrupt-priority arbitration, more than two vCPUs or a general vCPU scheduler;
- changes to virtio-blk persistence/durability, packed rings, `EVENT_IDX`, multi-queue storage or the independent persistent-flush surface;
- DMA/IOMMU, migration, resumable execution, whole-VM snapshots, throughput/latency claims or benchmark claims.

## Promotion rule

After guest-originated xAPIC IPI delivery is integrated and exact merged-`main` permanent workflows are green, seal the fixed APIC-ID1/vector-0x52 proof rather than farming additional targets or vectors.

The next SMP architecture audit should prefer a genuinely higher control-plane boundary. The strongest candidate is guest-driven AP startup through a bounded INIT/SIPI sequence that removes the current host `KVM_SET_MP_STATE(RUNNABLE)` shortcut and proves reset/startup state, trampoline execution and post-startup communication. If that cannot be made deterministic without a larger architecture change, choose another executable SMP/device interaction frontier rather than cloning the fixed IPI. Persistent backing/durability remains a separate storage frontier and should be advanced independently only when that surface is unclaimed.
