# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the currently integrated capability boundary and the selected next executable milestone.

## Current integrated state

The Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, bounded non-identity ELF64 virtual mapping, and bounded bidirectional userspace MMIO device execution are integrated on `main`.

The repository has typed, owned boundaries for KVM capability discovery, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, configured CPUID/MSR policy and state, vCPU register/special-register/MSR snapshots, centralized VM-exit dispatch, bounded one-vCPU execution, deterministic real-mode fixtures, bidirectional debug port I/O, typed MMIO exit servicing, one fixed byte-wide userspace MMIO device, one strict x86-64 long-mode bootstrap/execution path, and one bounded ELF64 loader that validates and materializes `PT_LOAD` segments including explicit BSS zeroing and a fixed non-identity alias mapping window.

Merged-main CI requires real-KVM evidence for the flat x86-64 long-mode proof, the non-identity ELF64 proof, and the real-mode bidirectional MMIO proof rather than treating `/dev/kvm` execution as optional evidence.

## Selected milestone — long-mode virtual MMIO composition

This milestone composes the integrated x86-64 page-table layer with the integrated userspace MMIO path. It deliberately proves one virtual device access end to end instead of broadening either the RAM-backed mapping API or the MMIO device model into a generic framework.

Acceptance contract:

- preserve `LongModePageMapping` unchanged as a RAM-backed alias contract whose physical pages remain validated inside low guest RAM;
- introduce a separate bounded long-mode MMIO layout rather than weakening the RAM-backed mapping invariant;
- preserve the existing low 2 MiB identity map, bootstrap page-table locations, long-mode control-register state, and current ELF alias behavior;
- install exactly one device PTE mapping guest virtual page `0x0050_0000` to guest-physical page `0x1000_0000` through the existing fixed alias PT;
- require the fixed device GPA to remain outside the configured slot-0 RAM and reject layout construction if caller RAM grows far enough to back that page;
- allow the existing exact byte device to be placed at an explicit GPA while preserving the established real-mode MMIO fixture default at `0x2000`;
- execute a reviewed 64-bit guest at identity-mapped RIP `0x10000` that uses a 64-bit address value for VA `0x500000`, writes `W`, reads the same virtual address and receives `R`, emits exactly `R64M` through port `0xe9`, and reaches HLT at RIP `0x1001e`;
- exact execution evidence must prove KVM reports MMIO write then read at translated GPA `0x10000000`, the device captures `W`, the guest consumes readback `R`, four debug-port exits spell `R64M`, and execution continues to terminal HLT after completion of the pending MMIO read;
- stable CI must retain the existing strict long-mode, non-identity ELF64, and real-mode MMIO proofs while independently requiring the long-mode virtual-MMIO proof;
- existing real-mode MMIO, RAM-backed alias, ELF64, port-I/O, state, and error contracts remain valid.

## Scope boundary

This milestone deliberately does **not** add:

- a general MMIO range registry or dynamic device registration;
- multiple MMIO devices or overlapping-range resolution;
- caller-defined or arbitrary virtual-MMIO mappings;
- wide/register-bank device semantics beyond the existing one-byte device;
- relaxation of `LongModePageMapping` physical-backing validation;
- dynamic page-table allocation or arbitrary virtual-address windows;
- PCI configuration space or bus enumeration;
- APIC, interrupt-controller, or interrupt-injection infrastructure;
- eventfd/ioeventfd/irqfd acceleration;
- virtio;
- DMA or IOMMU modeling;
- multiple RAM slots or memory hotplug;
- SMP;
- whole-VM snapshots;
- migration;
- resumable execution.

## Promotion rule

After long-mode virtual MMIO composition is integrated and exact merged-`main` CI is green, perform another architecture/integration audit before selecting the next frontier. The leading candidate is a small explicit MMIO range/device-routing layer only if it can be proven by an executable multi-device or range-routing integration slice; otherwise promote to the next independent architecture layer, likely interrupt/APIC foundation. Do not farm additional fixed-address path variants once virtual-MMIO composition is sealed.
