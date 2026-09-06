# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 execution, ELF64 loading/mapping, userspace and virtual MMIO, controller-backed interrupts, async timer delivery through direct GSI and irqfd/eventfd, ioeventfd signaling, guest-discovered PCI BAR MMIO, bounded virtio-rng/virtio-blk execution, and the two-vCPU SMP control plane through guest-driven INIT/SIPI, AP-owned real-mode-to-long-mode transition, guest-originated xAPIC IPI delivery, plus a separate bounded two-vCPU shared-memory work-dispatch data plane.

Current `main` is `815de7e06b388c64cdffe719816ed4144e5efcf1` through PR #104. The fixed INIT/SIPI/AP-long-mode/vector-`0x52` control plane from PRs #101–#103 remains independently integrated, and PR #104 added a distinct directly-initialized two-long-mode-vCPU mailbox protocol at GPA `0x9000` using locked byte `XCHG` command/acknowledgement ownership. Exact merged-main CI and the applicable permanent hosted-KVM workflows are green at this boundary.

The isolated one-item `0x21→0x42` work-dispatch phase is sealed. Do not farm another payload, polling constant, directly-initialized AP clone, alternate SIPI vector, alternate IPI vector, APIC ID, or vCPU2/vCPU3 merely to extend the phase number.

## Selected milestone — SIPI-started AP executes IPI-notified mailbox work

The next boundary is a cross-layer composition that removes the artificial separation between the integrated AP startup/control plane and the integrated mailbox data plane. In one VM and one execution, the BSP must start vCPU1 from `KVM_MP_STATE_UNINITIALIZED` through the existing guest xAPIC INIT/SIPI path, the AP must perform its existing guest-owned real-mode-to-long-mode transition, and the BSP must use the already integrated fixed vector `0x52` IPI to notify the running AP that one locked-`XCHG` mailbox work item is ready.

This is a composition of existing bounded contracts, not a third startup policy, general scheduler, scalable queue, or new interrupt-routing model. The fixed SIPI vector `0x08`, APIC ID `1`, LAPIC mapping, AP long-mode GDT/IDT layout, vector `0x52`, mailbox GPA `0x9000`, payload `0x21`, result `0x42`, and locked byte `XCHG` ownership transitions remain unchanged.

Acceptance contract:

- preserve ordinary CI, Rust 1.74 MSRV and every existing permanent hosted-KVM workflow, especially the independent INIT/SIPI, AP-long-mode, AP-long-mode-IPI and directly-initialized work-dispatch proofs;
- use one VM with exactly two vCPUs; vCPU1 begins `KVM_MP_STATE_UNINITIALIZED` and must be started by the guest BSP with the existing xAPIC INIT assert/deassert plus SIPI vector `0x08` sequence;
- after SIPI, vCPU1 must report the existing startup architectural state: MP runnable, RIP `0`, CS selector `0x0800`, CS base `0x8000`, CR0.PE clear, then perform its own PAE/CR3/EFER/CR0 transition into 64-bit mode;
- AP long-mode architectural state remains the integrated contract: stack `0x1ef000`, CS selector `0x08` with L=1, SS selector `0x10`, GDT `0x7000/0x17`, CR3 `0x1000`, required CR0/CR4/EFER bits, IDT `0x6000/0x52f`, and readiness with RFLAGS bit1 set and IF clear;
- mailbox GPA remains `0x9000`: payload at offset `0x00`, command at `0x08`, result at `0x10`, acknowledgement at `0x18`; payload is `0x21` and expected result is `0x42`;
- after AP readiness, BSP stores payload then publishes command `1` with a locked memory `XCHG`; old command ownership must be `0`, otherwise the guest follows its explicit `F` failure path;
- only after command publication may BSP send the existing guest-originated fixed xAPIC IPI vector `0x52` to APIC ID `1`;
- the AP must enter the existing vector-`0x52` handler, emit handler proof, issue LAPIC EOI and `iretq`; after return it claims command with locked `XCHG`, doubles payload, stores result `0x42`, publishes acknowledgement `1` with locked `XCHG`, and requires previous acknowledgement ownership `0`;
- BSP uses a bounded poll, consumes acknowledgement with locked `XCHG`, requires old acknowledgement `1`, validates result `0x42`, and leaves command and acknowledgement both cleared to zero;
- bounded poll exhaustion, ownership mismatch, startup/long-mode/IDT mismatch, IPI sequencing failure, mailbox mismatch, proof mismatch or terminal-state mismatch remain hard failures and must not be retried into success or hidden by changed expectations;
- exact BSP debug proof is `0IDSCXVD`; exact AP proof is `ALRIPD`; every byte-wide port-I/O exit must have exact direction, size, debug port, count and payload;
- exact final mailbox is payload `0x21`, command `0`, result `0x42`, acknowledgement `0`;
- exact BSP terminal report is `KVM_EXIT_HLT` at RIP `0x1009b`; exact AP terminal report is `KVM_EXIT_HLT` at RIP `0x80a6`; architectural RFLAGS bit1 is required on both and AP completion must retain IF set;
- KVM-aware integration independently validates both proof streams, all debug-port exits, initial/startup AP MP state, AP long-mode/IDT state, ready/completion RFLAGS, mailbox state and both terminal reports;
- permanent workflow `Strict KVM SIPI IPI work dispatch` must run independently on hosted KVM and require this exact composition while all previous permanent workflows remain unchanged;
- locally generated assembler/linker artifacts and construction scripts are not committed.

The implementation is currently in progress on `milestone/sipi-ipi-work-dispatch`. No capability is considered integrated until the exact candidate passes ordinary CI plus the new permanent hosted-KVM workflow, remains current with `main`, and completes the repository's normal review/merge audit.

## Scope boundary

This milestone deliberately does **not** add:

- a general scheduler, queue, multiple work items, multi-producer/multi-consumer protocol, futex/lock library or formal language-level memory model;
- alternate SIPI/IPI vectors, APIC IDs, repeated startup cycles, a third vCPU, CPU hotplug or a replacement startup path;
- AP-local periodic timer ownership, cross-vCPU TLB shootdown, per-CPU TSS, ring transitions or SYSCALL/SYSRET;
- new interrupt routing, MSI/MSI-X behavior, additional virtio/storage behavior, persistence/durability, DMA/IOMMU or migration;
- performance, latency, fairness, scalability or benchmark claims.

## Promotion rule

After the SIPI/IPI/mailbox composition is integrated and exact merged-`main` ordinary CI plus all permanent workflows are green, seal this fixed one-work-item composition rather than adding a second payload or alternate vector.

The next architecture audit should choose a genuinely new SMP capability that requires the now-unified startup/control/data plane. Strong candidates include AP-local timer ownership, a bounded cross-vCPU TLB-shootdown protocol after adding a meaningful shared virtual-memory invalidation need, or a privileged execution boundary such as per-CPU TSS/ring transition only when backed by an executable end-to-end proof. Persistent storage durability remains a separate frontier.
