# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the currently integrated capability boundary and the selected next executable milestone.

## Current integrated state

The Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, bounded bidirectional userspace MMIO device execution, and bounded long-mode virtual-MMIO composition are integrated on `main`.

The repository has typed, owned boundaries for KVM capability discovery, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, configured CPUID/MSR policy and state, vCPU register/special-register/MSR snapshots, centralized VM-exit dispatch, bounded one-vCPU execution, deterministic real-mode fixtures, bidirectional debug port I/O, typed MMIO exit servicing, one fixed byte-wide userspace MMIO device, one strict x86-64 long-mode bootstrap/execution path, one bounded ELF64 loader with a fixed non-identity alias window, and one fixed virtual-device mapping that translates VA `0x500000` to unbacked GPA `0x10000000` without weakening the RAM-backed alias invariant.

Merged-main CI requires real-KVM evidence for the flat x86-64 long-mode proof, the non-identity ELF64 proof, the real-mode bidirectional MMIO proof, and the long-mode virtual-MMIO proof rather than treating `/dev/kvm` execution as optional evidence.

## Selected milestone — bounded direct long-mode interrupt delivery

This milestone promotes the execution-control plane from polling/VM-exit-only progress to one explicit external-interrupt delivery path. It uses KVM's direct x86 vCPU vector-injection interface while the VMM remains in userspace interrupt-controller mode; it deliberately does not claim PIC, local-APIC, IOAPIC, IRQ-line, or routing semantics.

Acceptance contract:

- preserve all existing long-mode, ELF64, MMIO, port-I/O, CPU-policy, snapshot, and diagnostic contracts;
- add the x86 `KVM_INTERRUPT` vCPU ioctl through a tested fixed four-byte `kvm_interrupt` UAPI structure and preserve the requested vector exactly;
- do not create an in-kernel irqchip and do not change the current CPU policy that masks x2APIC, TSC-deadline, and KVM PV-unhalt while no LAPIC/IRQ-chip model exists;
- extend one bounded long-mode fixture with a real guest-memory GDT page at GPA `0x5000` and IDT page at GPA `0x6000`, while preserving the existing page-table bootstrap at `0x1000..0x5000`;
- install a present ring-0 64-bit code descriptor and one present interrupt gate for vector `0x40` targeting handler RIP `0x11000`;
- reject vectors in the x86 exception range, entry/handler collisions with reserved bootstrap/GDT/IDT pages, handlers outside the low identity map, and a bounded interrupt stack frame that would overlap reserved tables;
- queue vector `0x40` before first `KVM_RUN` while architectural IF is initially clear; the deterministic guest at RIP `0x10000` must execute `STI` plus one interrupt-shadow instruction, enter the handler, emit `I`, execute `IRETQ`, resume the interrupted main path, emit `M`, and execute HLT;
- exact executable evidence must therefore be `IM`, not merely handler entry: it proves interrupt delivery, handler execution, interrupt return, resumed guest execution, and terminal HLT;
- terminal proof requires HLT at RIP `0x10007`, architectural RFLAGS bit 1 set, and IF still set;
- stable CI must retain all four established strict real-KVM proofs and independently require the direct-interrupt proof;
- KVM ioctl failures remain named vCPU-operation errors; unsupported host/controller states are not swallowed or reinterpreted as successful injection.

## Scope boundary

This milestone deliberately does **not** add:

- `KVM_CREATE_IRQCHIP`, in-kernel PIC, local APIC, or IOAPIC state;
- IRQ pin/line semantics or `KVM_IRQ_LINE` routing;
- GSI routing, MSI/MSI-X, or PCI interrupt delivery;
- x2APIC, TSC-deadline timer, or PV-unhalt exposure;
- timer devices or periodic interrupt generation;
- multiple pending vectors, priority arbitration, nested-interrupt policy, or interrupt-window scheduling infrastructure;
- device-generated interrupt wiring from the existing MMIO device;
- multiple vCPUs or cross-vCPU interrupt routing;
- eventfd/ioeventfd/irqfd acceleration;
- arbitrary caller-supplied GDT/IDT layouts or guest-controlled descriptor-table construction;
- a general MMIO range/device registry;
- virtio;
- DMA or IOMMU modeling;
- multiple RAM slots or memory hotplug;
- SMP;
- whole-VM snapshots;
- migration;
- resumable execution.

## Promotion rule

After direct long-mode interrupt delivery is integrated and exact merged-`main` CI is green, perform another architecture/integration audit. The next frontier should move from a single directly injected vector toward a real interrupt-controller/device-delivery architecture only when the required KVM irqchip/APIC capability, CPU-feature exposure, routing semantics, state ownership, and executable proof can be introduced coherently. Do not farm additional fixed direct-vector variants once the `IM` handler/return proof is sealed.
