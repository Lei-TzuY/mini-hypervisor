# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 execution, ELF64 loading/mapping, userspace and virtual MMIO, controller-backed interrupts, async timer delivery through direct GSI and irqfd/eventfd, ioeventfd signaling, guest-discovered PCI BAR MMIO, bounded virtio-rng/virtio-blk execution, and a two-vCPU SMP control plane with targeted MSI, guest-originated fixed IPI, guest-driven INIT/SIPI startup, and a SIPI-started AP that performs its own real-mode-to-long-mode transition.

Current `main` is `b4997721ef02a4e43d05bcb135d33dc56d32acbe` through PR #102. The AP still begins as `KVM_MP_STATE_UNINITIALIZED`, is started by BSP xAPIC INIT assert/deassert plus SIPI vector `0x08`, consumes the established KVM startup `EAGAIN` handoff, proves startup `CS=0x0800`, base `0x8000`, `RIP=0`, `CR0.PE=0`, then guest code installs its own GDT, enables PAE, loads `CR3=0x1000`, enables EFER.LME and CR0.PE|PG, far-jumps to 64-bit selector `0x08`, selects stack `0x1ef000`, writes marker `K`, and proves `ALPD`. Exact merged-main CI and the permanent INIT/SIPI and AP-long-mode workflows are green at this boundary.

The fixed AP startup and guest-owned mode-transition phase is sealed. Do not farm alternate SIPI vectors, GDT addresses, stack values, repeated INIT/SIPI cycles, or vCPU2/vCPU3 clones merely to extend the phase number.

## Selected milestone — guest-originated IPI into the SIPI-started long-mode AP

The next SMP boundary is AP-owned 64-bit interrupt handling. Reuse the exact shared INIT/SIPI orchestration and guest-owned AP long-mode transition, then give the AP a bounded IDT gate and have BSP guest code deliver one fixed xAPIC IPI only after the AP reports 64-bit readiness with IF clear. No host interrupt injection may stand in for the guest ICR write.

Acceptance contract:

- preserve `run_two_vcpu_init_sipi()` and `run_two_vcpu_ap_long_mode()` plus their permanent real-KVM proofs;
- extend the existing startup orchestration rather than create a second VM/startup flow;
- vCPU1 must still begin `KVM_MP_STATE_UNINITIALIZED`, consume the existing startup `EAGAIN`, and prove the same real-mode checkpoint before guest trampoline execution;
- userspace must not call `initialize_long_mode()` or rewrite AP CR0/CR3/CR4/EFER/CS into the desired final state;
- AP guest code retains the #102 transition contract: GDT base `0x7000`, limit `0x17`, code selector `0x08`, data selector `0x10`, `RSP=0x1ef000`, `CR3=0x1000`, CR0 PE|PG, CR4 PAE, EFER LME|LMA;
- install exactly one bounded AP IDT gate for vector `0x52`, selector `0x08`, handler GPA `0x12000`; AP guest code loads IDTR base `0x6000`, limit `0x52f` after entering long mode;
- AP readiness proof byte `R` must be observed with architectural RFLAGS bit1 set and IF clear;
- BSP may send the fixed IPI only after AP readiness, by writing the existing guest-visible xAPIC ICR high/low path for destination APIC ID 1 and vector `0x52`; host `KVM_INTERRUPT`, `KVM_SIGNAL_MSI`, GSI injection, or another direct host delivery path is not acceptable evidence;
- AP uses adjacent `sti; hlt`, enters the vector-`0x52` 64-bit handler, emits `I`, writes LAPIC EOI, returns with `iretq`, resumes mainline `M`, writes shared marker `K`, emits completion `D`, and finishes with IF set;
- exact BSP proof is `0IDSXMD`; exact AP proof is `ALRIMD`; every byte-wide debug-port exit must match direction, size, port and payload;
- final AP `KVM_MP_STATE` remains RUNNABLE; final long-mode GDT/segment/control-register state must retain the #102 contract; final IDTR must be `0x6000/0x52f` and final RFLAGS must have architectural bit1 and IF set;
- KVM-aware integration independently validates startup checkpoint, both proof streams, marker, AP long-mode state, IDTR/readiness state, completion RFLAGS, and every debug-port exit;
- provide a standalone `two-vcpu-ap-long-mode-ipi` executable and a permanent `Strict KVM two-vCPU AP long-mode IPI` hosted-KVM workflow without weakening or replacing the historical INIT/SIPI and AP-long-mode workflows;
- deterministic machine-code/IDT layout remains covered by unit regression; generated assembler/linker artifacts and construction scripts are not committed;
- INIT/SIPI, EAGAIN handoff, guest long-mode transition, IDT installation, ICR route, interrupt/EOI/IRETQ, proof, shared marker, MSRV or real-KVM failures remain hard failures and must not be skipped, retried into success, or hidden by changed expectations.

Executable evidence has been established on the implementation branch before this roadmap synchronization: CI #623 completed successfully, the historical INIT/SIPI and AP-long-mode permanent workflows remained green, and `Strict KVM two-vCPU AP long-mode IPI` run #3 completed successfully on hosted KVM. That proof reported startup `CR0=0x60000010`, final `CR0=0xe0000011`, `CR3=0x1000`, `CR4=0x20`, `EFER=0x500`, IDTR `0x6000/0x52f`, readiness RFLAGS `0x86` with IF clear, completion RFLAGS `0x286` with IF set, marker `K`, BSP proof `0IDSXMD`, and AP proof `ALRIMD`. Because this documentation commit changes the candidate head, the final exact candidate must rerun all applicable CI/permanent workflows before integration.

## Scope boundary

This milestone deliberately does **not** add:

- x2APIC, logical destination mode, IOAPIC/MSI routing changes, arbitrary vector registration or arbitrary IPI routing;
- multiple APs, firmware/ACPI discovery, AP hotplug or repeated INIT/SIPI cycles;
- per-CPU TSS, ring transitions, SYSCALL/SYSRET, scheduler or general SMP runtime;
- a formal shared-memory memory model, futexes, generic atomics framework or lock implementation;
- AP-local timer ownership, periodic timer scheduling or timekeeping;
- additional virtio/storage functionality, persistence/durability, DMA/IOMMU or migration;
- performance, latency, scalability or benchmark claims.

## Promotion rule

After the AP-owned 64-bit IPI path is integrated and exact merged-`main` CI plus all relevant permanent workflows are green, seal the fixed vector-`0x52`/APIC-ID1 proof rather than cloning more vectors.

The next SMP audit should choose a materially higher interaction boundary. Prefer a bounded two-vCPU shared-memory synchronization/work-dispatch primitive with executable ordering evidence, an AP-local timer/interrupt workload if it introduces independent execution ownership, or another cross-vCPU 64-bit workload that requires both processors to make progress. Do not claim a general scheduler, memory model or SMP scalability result without corresponding implementation and evidence. Persistent storage durability remains a separate frontier.
