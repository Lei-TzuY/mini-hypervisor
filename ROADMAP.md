# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO execution, long-mode virtual-MMIO composition, direct long-mode interrupt delivery, one in-kernel x86 irqchip/GSI route, MMIO-device interrupt delivery, stateful device-owned level-interrupt lifecycle, bounded two-device MMIO registration/mapping, two independently routed MMIO level-interrupt sources, and one genuinely host-driven asynchronous timer wakeup.

The asynchronous timer phase is integrated at commit `e2c0f1c7686e39a31949e038d1d6ba7d4bf70746` through PR #86. Exact merged-main CI #389 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all ten earlier strict real-KVM gates, and the eleventh strict async-timer gate. Its executable proof keeps IF clear through readiness `R` and arm barrier `A`, uses adjacent `sti; hlt`, receives a host-worker GSI0 edge through the existing PIC/LAPIC ExtINT route, emits handler byte `T`, resumes the halted mainline with `W`, and reaches terminal userspace barrier `D`. Exact proof is `RATWD`; arm RFLAGS has architectural bit 1 set with IF clear, while completion has bit 1 and IF set.

That direct worker-ioctl timer phase is sealed. Do not farm fixed delay variants, more one-shot timer instances, or additional direct `KVM_IRQ_LINE` workers merely to extend the phase number.

## Selected milestone — KVM irqfd accelerated timer delivery

The next architecture boundary is kernel-assisted event delivery. The integrated async timer still requires its worker thread to own a duplicated KVM VM fd and issue `KVM_IRQ_LINE`. This milestone keeps the exact same deterministic guest, PIC/ExtINT path, `cli` arm barrier, race-safe `sti; hlt` handoff, handler, watchdog, and `RATWD` verifier, but changes only the delivery transport: userspace registers an eventfd against GSI0 with `KVM_IRQFD`, and the timer worker signals only that eventfd.

This is deliberately one bounded edge-triggered irqfd proof. It is not a claim of irqfd resample/level semantics, ioeventfd acceleration, arbitrary routing, or a general event framework.

Acceptance contract:

- preserve all eleven integrated strict real-KVM gates and every existing long-mode, ELF64, MMIO, interrupt, snapshot, CPU-policy, diagnostic, and Rust 1.74 MSRV contract;
- require `KVM_CAP_IRQFD` as a hard runtime capability for the irqfd executable and hosted strict gate; absence must fail that gate rather than silently skip or fall back to direct `KVM_IRQ_LINE`;
- implement the exact Linux `struct kvm_irqfd` shape used by this repository: eventfd descriptor, GSI, flags, resamplefd, and padding totaling 32 bytes; assignment uses flags zero and deassignment uses `KVM_IRQFD_FLAG_DEASSIGN`;
- create eventfd with `EFD_CLOEXEC | EFD_NONBLOCK`, wrap every successful descriptor immediately in `OwnedFd`, and signal with one exact native-endian `u64` value of 1 while treating short writes and non-EINTR failures as hard errors;
- complete every fallible local eventfd preparation step before changing kernel irqfd state: create the registration eventfd and duplicate the worker signal handle first, then issue `KVM_IRQFD assign`; once assignment succeeds, no later signal-handle duplication may create an early-return cleanup gap;
- preflight the fail-closed watchdog duplicated VM-fd handle before any irqfd registration is established, so watchdog setup failures cannot strand a registered irqfd;
- establish a known inactive GSI0 level before assignment, register the eventfd to GSI0 before entering the potentially blocking guest `sti; hlt`, then let the timer worker own only the duplicated eventfd signal handle; the irqfd timer worker must not own a VM fd, call `KVM_IRQ_LINE`, or alias guest RAM;
- retain the direct-GSI watchdog only as an anti-hang mechanism. Any watchdog intervention remains a hard failure and cannot manufacture acceptable irqfd evidence;
- on every non-hanging path after a successful assignment, cancel/join workers and explicitly issue `KVM_IRQFD_FLAG_DEASSIGN` before accepting timer/proof results; deassignment failure is a hard failure;
- reuse the exact deterministic guest proof `RATWD`: readiness `R`, arm barrier `A` with IF clear, vector `0x40` handler `T` plus master-PIC EOI and `iretq`, resumed mainline `W`, and terminal userspace barrier `D`;
- preserve semantic LAPIC state: SPIV software-enable remains set and LINT0 remains unmasked ExtINT; arm RFLAGS requires architectural bit 1 with IF clear and completion requires bit 1 with IF set;
- KVM-aware integration must independently execute the irqfd transport and validate GSI/vector, LAPIC state, arm/completion RFLAGS, all five byte-wide debug-port exits, and exact `RATWD` proof;
- stable CI must retain all eleven integrated strict real-KVM gates unchanged and add an independent twelfth irqfd timer gate requiring the `KVM_CAP_IRQFD` executable, GSI0/vector0x40, semantic LAPIC state, IF-clear arm point, IF-enabled completion, and proof bytes `[82, 65, 84, 87, 68]`;
- final candidate evidence must come from an exact PR run whose recorded job steps actually include and pass the twelfth irqfd gate; an earlier green run that omits that step is not sufficient merge evidence;
- capability failure, eventfd creation/duplication/signal failure, irqfd assign/deassign failure, worker panic, watchdog intervention, watchdog failure, unexpected VM exit, wrong proof order, wrong PIC/LAPIC state, or wrong RFLAGS remain hard failures and must not be swallowed, retried into success, or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** add:

- irqfd resample/level-triggered semantics, `KVM_CAP_IRQFD_RESAMPLE`, shared GSIs, interrupt coalescing policy, or irqfd lifecycle generalization beyond this one executable edge route;
- `KVM_IOEVENTFD`, MMIO/PIO doorbell acceleration, arbitrary `KVM_SET_GSI_ROUTING`, IOAPIC programming, MSI/MSI-X, x2APIC, or slave-PIC expansion;
- periodic or programmable timers, PIT/HPET emulation, local-APIC timer programming, TSC-deadline, timer wheels, or a general scheduler;
- realtime/wall-clock latency guarantees, controlled performance benchmarks, or host scheduling claims;
- PCI/PCIe configuration space, BARs, virtio transport, DMA, IOMMU, or device hotplug;
- guest-memory sharing with a worker, multi-vCPU delivery, SMP, migration, resumable execution, or whole-VM snapshots.

## Promotion rule

After irqfd timer delivery is integrated and exact merged-`main` CI is green, seal the first irqfd acceleration proof rather than multiplying eventfds or fixed GSIs.

The next architecture audit should choose another materially new interaction boundary. Strong candidates are a minimal `KVM_IOEVENTFD`-backed guest doorbell path only if it closes an executable device event/interrupt round trip, or a minimal PCI/virtio transport that introduces real discovery/configuration semantics. IOAPIC/MSI, SMP, DMA/IOMMU, migration, irqfd resample semantics, and performance work remain separate higher-order frontiers.
