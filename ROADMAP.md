# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the integrated capability boundary and the selected next executable milestone.

## Current integrated state

`main` contains the Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO execution, long-mode virtual-MMIO composition, direct long-mode interrupt delivery, one in-kernel x86 irqchip/GSI route, MMIO-device interrupt delivery, stateful device-owned level-interrupt lifecycle, bounded two-device MMIO registration/mapping, two independently routed MMIO level-interrupt sources, one host-driven asynchronous timer wakeup, and one KVM irqfd/eventfd accelerated timer transport.

The irqfd timer phase is integrated at commit `a896487a7adf67cfebfad8b0241cb6302aa2090e` through PR #87. Exact merged-main CI #395 completed successfully with format, Clippy, tests, build, rustdoc, Rust 1.74 MSRV, all eleven earlier strict real-KVM gates, and the twelfth strict irqfd timer gate. It preserves the direct timer guest contract `RATWD` while changing only host-to-guest delivery: the worker signals an eventfd assigned to GSI0 through `KVM_IRQFD`; explicit deassignment is required before proof acceptance; the fail-closed direct-GSI watchdog cannot manufacture passing evidence.

That first irqfd acceleration phase is sealed. Do not farm more fixed eventfds, delay variants, or GSI clones merely to extend the phase number.

## Selected milestone — KVM ioeventfd to irqfd accelerated round trip

The next architecture boundary is a bidirectional kernel-assisted event transport. Existing accelerated timer delivery covers host-to-guest notification through `KVM_IRQFD`, while guest-to-host device notification still normally surfaces as a userspace `KVM_EXIT_MMIO`. This milestone combines the existing bounded virtual-MMIO mapping with `KVM_IOEVENTFD`: one guest MMIO doorbell write is consumed by KVM and signals an eventfd without a userspace MMIO exit; a userspace bridge worker consumes exactly one doorbell event and signals an already registered irqfd eventfd; the same GSI0/PIC/LAPIC ExtINT path enters vector `0x40`, executes the handler, and resumes the guest.

This is deliberately one bounded doorbell round trip, not a PCI/virtio transport or general event framework.

Acceptance contract:

- preserve all twelve integrated strict real-KVM gates and every existing long-mode, ELF64, MMIO, interrupt, snapshot, CPU-policy, diagnostic, and Rust 1.74 MSRV contract;
- require both `KVM_CAP_IOEVENTFD` and `KVM_CAP_IRQFD` as hard capabilities for the executable/hosted proof; absence must fail rather than silently skip or fall back;
- implement the exact Linux 64-byte `struct kvm_ioeventfd` ABI used here: `datamatch`, MMIO address, length, signed fd, flags and 36-byte padding; use `KVM_IOEVENTFD_FLAG_DATAMATCH` for assignment and add `KVM_IOEVENTFD_FLAG_DEASSIGN` for explicit removal;
- reuse the bounded long-mode MMIO alias VA `0x500000` mapped to unbacked GPA `0x10000000`; the deterministic doorbell is one byte with exact datamatch `0x5a`;
- create/duplicate all local eventfds and preflight the fail-closed watchdog before either accelerated registration can be stranded; establish GSI0 inactive before assignment;
- assign irqfd and ioeventfd before the guest doorbell executes; if ioeventfd assignment fails after irqfd succeeded, explicitly deassign irqfd before returning;
- the bridge worker owns only duplicated eventfd descriptors, never a KVM VM fd or guest RAM; it must consume exactly one ioeventfd counter event and only then signal the irqfd eventfd;
- re-entering the guest after readiness `R` must execute the registered MMIO write entirely inside KVM. Reaching arm barrier `A` without a `KVM_EXIT_MMIO` proves the guest-to-host side used ioeventfd instead of userspace MMIO emulation;
- arm barrier `A` remains under CLI with architectural RFLAGS bit 1 set and IF clear; immediately following instructions are adjacent `sti; hlt`, preserving the race-safe interrupt handoff used by the integrated timer phases;
- irqfd delivery must enter vector `0x40`, emit handler byte `T`, issue master-PIC EOI, `iretq`, resume mainline byte `W`, then reach terminal userspace barrier `D`; exact proof remains `RATWD`;
- the fail-closed direct-GSI watchdog exists only to prevent a broken bridge/irqfd path from wedging CI; any watchdog intervention is a hard failure and cannot count as round-trip evidence;
- on every non-hanging path after successful registration, cancel/join workers and issue both `KVM_IOEVENTFD` deassignment and `KVM_IRQFD_FLAG_DEASSIGN` before worker/proof results can be accepted; cleanup failures remain hard failures;
- preserve semantic LAPIC state: SPIV software-enable set and LINT0 unmasked ExtINT; completion RFLAGS requires architectural bit 1 and IF set;
- KVM-aware integration must independently validate doorbell GPA/value, exactly one bridge event, GSI/vector, LAPIC state, arm/completion RFLAGS, all five byte-wide debug-port exits and exact `RATWD` proof;
- stable CI must retain all twelve integrated strict real-KVM gates unchanged and add an independent thirteenth round-trip gate requiring `KVM_CAP_IOEVENTFD`, `KVM_CAP_IRQFD`, doorbell GPA `0x10000000`, value `90`, event count `1`, GSI0/vector0x40, semantic LAPIC ExtINT state, IF-clear arm point, IF-enabled completion and proof bytes `[82, 65, 84, 87, 68]`;
- capability, eventfd, poll/read/signal, assignment/deassignment, bridge worker, watchdog, unexpected VM exit, MMIO fallback, proof order, PIC/LAPIC, or RFLAGS failures remain hard and must not be swallowed, retried into success, or hidden by changed expectations.

## Scope boundary

This milestone deliberately does **not** add:

- ioeventfd PIO mode, zero-length any-write matching, `KVM_IOEVENTFD_FLAG_FAST_MMIO`, multiple queue doorbells, shared eventfds, batching, or notification coalescing;
- irqfd resample/level semantics, `KVM_CAP_IRQFD_RESAMPLE`, shared GSIs, arbitrary `KVM_SET_GSI_ROUTING`, IOAPIC/MSI/MSI-X, or x2APIC;
- PCI/PCIe configuration space, BAR enumeration, virtio queue/configuration transport, DMA, IOMMU, or device hotplug;
- periodic/programmatic timers, PIT/HPET/APIC timer, scheduler framework, realtime latency guarantees, or performance benchmark claims;
- guest-memory sharing with workers, multi-vCPU delivery, SMP, migration, resumable execution, or whole-VM snapshots.

## Promotion rule

After the ioeventfd-to-irqfd round trip is integrated and exact merged-`main` CI is green, seal this fixed doorbell bridge rather than multiplying eventfd flags, addresses, or queues.

The next architecture audit should prefer a materially higher device-model frontier. A strong candidate is a minimal PCI/virtio discovery/configuration transport that gives the accelerated doorbell a real enumerated device/queue context and executable guest-visible semantics. IOAPIC/MSI, SMP, DMA/IOMMU, migration, irqfd resample, and performance work remain separate higher-order phases unless an executable prerequisite makes one of them necessary first.
