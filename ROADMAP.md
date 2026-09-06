# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 loading/mapping, userspace and virtual MMIO, direct/controller-backed interrupt delivery, MMIO interrupt lifecycles and multi-device routing, host-driven timer delivery through direct `KVM_IRQ_LINE` and irqfd/eventfd, ioeventfd-backed device signaling, guest-discovered PCI BAR MMIO, bounded virtio-rng split-ring execution with INTx/MSI completion, bounded virtio-blk read/INTx execution, same-VM in-memory virtio-blk `T_OUT`→`T_IN` write/readback, bounded four-sector multi-sector read/write, negotiated virtio-blk indirect descriptor execution, a bounded two-vCPU same-VM shared-memory handoff, host-targeted fixed MSI delivery to a uniquely thread-owned second vCPU, and guest-originated xAPIC fixed-IPI delivery from vCPU0 to vCPU1.

Current `main` is `4cf0afb321196bd81c4977d65d8638a457e3d0b1` through PR #100. PR #99 established explicit host promotion/readback of vCPU1 as `KVM_MP_STATE_RUNNABLE` plus host `KVM_SIGNAL_MSI` targeting physical APIC ID 1. PR #100 then moved interrupt origination into the guest: vCPU0 maps the local APIC, writes ICR high/low for APIC ID 1 and vector `0x52`, vCPU1 receives the fixed IPI in its uniquely owned worker thread, and the handler acknowledges LAPIC EOI before returning. The permanent CI, storage, two-vCPU foundation, targeted-MSI and guest-IPI workflows are green on this integrated boundary.

The host-targeted-MSI and guest-originated fixed-IPI phases are sealed. Do not farm more fixed MSI/IPI vectors, destination APIC IDs, or vCPU2/vCPU3 clones merely to extend the phase number. The independently named persistent-flush storage surface remains a separate storage frontier.

## Selected milestone — guest-driven xAPIC INIT/SIPI startup of the second vCPU

The next multiprocessor boundary is application-processor lifecycle control. The integrated two-vCPU proofs currently use a userspace `KVM_SET_MP_STATE(RUNNABLE)` shortcut before guest execution. This milestone removes that shortcut from its fixture: vCPU1 must begin `KVM_MP_STATE_UNINITIALIZED`, vCPU0 must send a bounded xAPIC INIT assert/deassert plus SIPI sequence to APIC ID 1, and Linux KVM must establish the SIPI-selected real-mode startup state before the AP executes a trampoline at GPA `0x8000`.

Linux KVM's observable startup boundary matters here. `KVM_GET_MP_STATE` by itself does not process pending LAPIC startup events, so this milestone does **not** claim that userspace can sample `INIT_RECEIVED` after each BSP debug-port barrier. Instead, after the BSP has committed all three ICR commands, the first AP `KVM_RUN` must take the x86 UNINITIALIZED-vCPU startup path, consume pending INIT/SIPI, and return the documented `EAGAIN`/`WouldBlock` handoff. At that exact checkpoint userspace must observe `MP_STATE=RUNNABLE`, `RIP=0`, `CS.selector=0x0800`, `CS.base=0x8000`, and `CR0.PE=0`; only then may one subsequent `KVM_RUN` execute the trampoline.

Acceptance contract:

- preserve current main CI, every permanent virtio-blk workflow, the two-vCPU foundation, targeted-MSI and guest-IPI workflows, all existing long-mode/ELF64/MMIO/interrupt/PCI/virtio contracts, and Rust 1.74 MSRV;
- do not modify or compete with independently owned storage/virtio surfaces;
- do not call `ensure_runnable_mp_state()` or otherwise set vCPU1 RUNNABLE from userspace in this fixture;
- create vCPU1 through the in-kernel irqchip path and prove its initial read-only MP state is exactly `KVM_MP_STATE_UNINITIALIZED (1)`;
- vCPU0 must map the local APIC through the existing bounded virtual-MMIO mechanism and target physical APIC ID 1 with ICR high `0x01000000`;
- vCPU0 must issue INIT assert `0x0000c500`, INIT deassert `0x00008500`, then SIPI `0x00000608`; the SIPI vector is exactly `0x08` and therefore selects real-mode trampoline base `0x8000`;
- the BSP debug-port proof is exactly `0IDSMD`: `0` is the pre-INIT barrier, `I` follows INIT assert, `D` follows INIT deassert, `S` follows SIPI, `M` proves the BSP observed the AP's guest-memory marker, and the final `D` is the BSP completion barrier;
- after those BSP commands commit, move vCPU1 into exactly one worker thread; no `Vcpu: Sync`, shared concurrent vCPU access or scheduler abstraction is introduced;
- the AP's first `KVM_RUN` must return exactly the startup `EAGAIN`/`WouldBlock` handoff; EINTR may be restarted as usual, but any guest exit, other error, or repeated EAGAIN after the checkpoint remains a hard failure rather than a retry loop;
- immediately after that startup handoff require `KVM_MP_STATE_RUNNABLE (0)`, `RIP=0`, `CS.selector=0x0800`, `CS.base=0x8000`, and `CR0.PE=0` before any trampoline instruction executes;
- the one subsequent AP `KVM_RUN` must enter the trampoline, which keeps IF clear, emits exact proof `APD`, writes marker `K` to GPA `0x9000`, and reaches an explicit completion barrier;
- BSP guest code must observe marker `K` from shared guest RAM before its `M`/`D` completion bytes; userspace-only handoff is insufficient;
- final AP MP state remains RUNNABLE, AP RFLAGS architectural bit 1 remains set and IF remains clear;
- KVM-aware integration independently validates the startup checkpoint, both proof streams, both debug-port exit sequences, marker value, final MP state and RFLAGS;
- the permanent `Strict KVM two-vCPU INIT SIPI` workflow must require vector `0x08`, trampoline GPA `0x8000`, initial MP state 1, startup MP state 0, startup RIP 0, startup CS selector/base `0x0800`/`0x8000`, CR0.PE clear, final MP state 0, marker `K`, proofs `0IDSMD`/`APD`, and AP completion RFLAGS bit1 set with IF clear;
- the EAGAIN handling remains fixture-specific: generic `Vcpu::run_once()` must not start swallowing or indefinitely retrying `WouldBlock` failures for unrelated execution paths;
- INIT/SIPI encoding, startup-state readback, worker ownership, shared-memory handoff, proof ordering, RFLAGS, MSRV or real-KVM failures remain hard failures and must not be skipped, slept into success, retried into success or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** add:

- x2APIC, logical destination mode, broadcast/shorthand INIT/SIPI or more than one AP;
- an AP long-mode transition, per-CPU GDT/IDT/TSS setup, ACPI/MADT discovery, firmware AP startup tables or a general trampoline allocator;
- hotplug, repeated reset/startup cycles, NMI/SMI startup variants or a general vCPU scheduler;
- changes to virtio-blk persistence/durability, packed rings, `EVENT_IDX`, multi-queue storage or the independent persistent-flush surface;
- DMA/IOMMU, migration, resumable execution, whole-VM snapshots, throughput/latency claims or benchmark claims.

## Promotion rule

After guest-driven INIT/SIPI startup is integrated and exact merged-`main` permanent workflows are green, seal the fixed APIC-ID1/vector-0x08 real-mode trampoline proof rather than farming alternate SIPI vectors or repeated startup cycles.

The next SMP architecture audit should prefer a higher execution boundary. A strong candidate is a bounded AP real-mode-to-long-mode transition that reuses the established SIPI lifecycle but proves a second vCPU can join the existing 64-bit execution environment with its own validated stack/state before participating in shared-memory or interrupt work. If that cannot be made coherent without a larger per-vCPU descriptor/state architecture, choose another executable SMP/device interaction frontier rather than cloning the fixed startup fixture. Persistent backing/durability remains a separate storage frontier and should advance independently only when that surface is unclaimed.
