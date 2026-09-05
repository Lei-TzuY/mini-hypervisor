# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO device execution, bounded long-mode virtual-MMIO composition, and bounded direct long-mode interrupt delivery through the userspace `KVM_INTERRUPT` window handshake.

The repository has typed, owned boundaries for KVM capability discovery, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, configured CPUID/MSR policy and state, vCPU register/special-register/MSR snapshots, centralized VM-exit dispatch, bounded one-vCPU execution, deterministic real-mode fixtures, bidirectional debug port I/O, typed MMIO servicing, one fixed byte-wide userspace MMIO device, one strict x86-64 long-mode bootstrap/execution path, one bounded ELF64 loader with a fixed non-identity alias window, one fixed virtual-device mapping, and one fixed GDT/IDT external-vector layout.

Merged-main CI requires real-KVM executable evidence for the long-mode, ELF64, real-mode MMIO, long-mode virtual-MMIO, and direct-interrupt proofs rather than treating `/dev/kvm` execution as optional evidence.

## Selected milestone — bounded in-kernel irqchip GSI delivery

This milestone promotes the interrupt-control plane from direct userspace vector injection to one real controller-backed legacy route. The candidate creates KVM's in-kernel x86 irqchip before any vCPU exists, explicitly owns the BSP LAPIC LINT0 ExtINT state required for legacy PIC delivery, pulses GSI0 through `KVM_IRQ_LINE`, and proves guest handler entry plus return to the interrupted main path.

Acceptance contract:

- preserve all established long-mode, ELF64, MMIO, direct-interrupt, CPU-policy, snapshot, exit-diagnostic, and strict real-KVM contracts;
- require `KVM_CAP_IRQCHIP` only for this controller-backed path rather than making it a global backend requirement;
- issue `KVM_CREATE_IRQCHIP` before `KVM_CREATE_VCPU` and treat creation failure as a named hard error;
- expose tested fixed UAPI boundaries for `kvm_irq_level`, `KVM_IRQ_LINE`, the 0x400-byte x86 `kvm_lapic_state`, `KVM_GET_LAPIC`, and `KVM_SET_LAPIC`;
- initialize the guest's master/slave 8259 PICs, remap master IRQ0 to vector `0x40`, unmask only IRQ0 on the master, and mask every slave IRQ;
- reuse the established GDT/IDT interrupt layout with vector `0x40` targeting the fixed handler at `0x11000`;
- read the current LAPIC state, preserve unrelated state, set only the BSP software-enable bit plus LINT0 delivery mode/mask fields needed for unmasked ExtINT delivery, write the state, then read it back and require software APIC enabled, LINT0 ExtINT, and LINT0 unmasked before publishing success;
- never treat a serviceable `KVM_EXIT_IO` RIP as a portable architectural commit point: the guest emits `R` then a second `A` I/O barrier, and userspace pulses GSI0 only after the second exit and after verifying architectural RFLAGS bit 1 plus IF;
- pulse fixed GSI0 as an edge by asserting level 1 and then deasserting level 0; ioctl failure on either half is a hard failure;
- after the pulse, require the next proof byte to be handler byte `I`; the handler sends a non-specific EOI to the master PIC and executes `IRETQ`;
- require resumed main execution to emit `M`, followed by a final `D` completion barrier. Reaching `D` proves an additional `KVM_RUN` occurred after the `M` exit, so resumed-main output is committed without relying on serviceable-I/O RIP semantics;
- do not require `KVM_EXIT_HLT` as this path's terminal evidence: with an in-kernel LAPIC, x86 KVM may keep an HLT vCPU non-runnable inside the kernel. The host intentionally stops after the `D` I/O completion barrier and does not re-enter the safety-fallback HLT;
- exact executable proof is therefore `RAIMD`, together with LAPIC readback evidence and IF set both before the GSI pulse and at completion;
- stable CI must retain all five established strict real-KVM proofs and independently require this sixth controller-backed proof. KVM-aware integration tests must assert exact byte-wide I/O metadata and the LAPIC/RFLAGS contract;
- unsupported capability, irqchip creation failure, LAPIC read/write/readback failure, unexpected I/O sequence, GSI assert/deassert failure, or inconsistent architectural state must remain visible as failure and must not be skipped, retried into success, or reinterpreted as evidence.

The candidate has already demonstrated the strict contract on a real hosted KVM runner with GSI `0`, vector `0x40`, LAPIC SPIV `0x1ff`, LINT0 `0x700`, armed/completion RFLAGS `0x202`, and proof `RAIMD`. Integration remains gated on the final reviewable exact head and merged-`main` CI.

## Scope boundary

This milestone deliberately does **not** add:

- caller-defined `KVM_SET_GSI_ROUTING` tables or arbitrary GSIs;
- guest-programmed IOAPIC delivery, IOAPIC redirection-table ownership, or a general interrupt-router abstraction;
- MSI/MSI-X or PCI interrupt delivery;
- PIT, HPET, LAPIC timer, TSC-deadline timer, periodic interrupt generation, or timer scheduling;
- x2APIC exposure or a general APIC state API;
- multiple pending interrupts, priority arbitration, nested-interrupt policy, or cross-vCPU routing;
- device-generated interrupts from the existing MMIO device;
- eventfd/ioeventfd/irqfd acceleration;
- multiple vCPUs, SMP, whole-VM snapshots, migration, or resumable execution;
- arbitrary caller-supplied GDT/IDT layouts or guest-controlled descriptor-table construction;
- a general MMIO range/device registry, virtio, DMA, IOMMU modeling, multiple RAM slots, or memory hotplug.

## Promotion rule

After the irqchip/GSI milestone is merged and exact merged-`main` CI is green, perform another architecture/integration audit and seal this controller phase. Do not farm more fixed GSI/PIC/vector variants.

The next preferred executable frontier is **device-generated interrupt composition**: connect the existing userspace MMIO device's explicit state transition to the established fixed GSI/controller path so one guest MMIO operation can cause a host-owned device event, a single GSI edge, PIC/LAPIC delivery, handler/EOI/IRETQ, resumed guest execution, and a deterministic completion proof. That slice must define one-shot event ownership, deassertion/duplicate rules, and failure propagation before implementation; it must not expand into a general device bus, arbitrary routing, timers, irqfd, or SMP merely to claim a larger architecture.
