# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the currently integrated capability boundary and the selected next executable milestone.

## Current integrated state

The Phase 73 foundation, deterministic x86-64 long-mode execution, bounded ELF64 `ET_EXEC` loading/execution, and bounded non-identity ELF64 virtual mapping are integrated on `main`.

The repository has typed, owned boundaries for KVM capability discovery, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, configured CPUID/MSR policy and state, vCPU register/special-register/MSR snapshots, centralized VM-exit dispatch, bounded one-vCPU execution, deterministic real-mode fixtures, bidirectional debug port I/O, one strict x86-64 long-mode bootstrap/execution path, and one bounded ELF64 loader that validates and materializes `PT_LOAD` segments including explicit BSS zeroing and a fixed non-identity alias mapping window.

Merged-main CI requires real-KVM evidence for both the flat x86-64 long-mode proof and the non-identity ELF64 proof rather than treating `/dev/kvm` execution as optional evidence.

## Selected milestone — bounded bidirectional MMIO device execution

This milestone promotes the one-vCPU execution loop from port-I/O-only device servicing to one explicit userspace MMIO path while remaining deliberately smaller than a general device framework.

Acceptance contract:

- classify Linux KVM `KVM_EXIT_MMIO` as a typed vCPU exit without disturbing existing exit classifications;
- decode the fixed x86 `kvm_run` MMIO payload only after validating the current exit reason, direction, and `len` in `1..=8`;
- copy MMIO write bytes into owned Rust state and never expose stale `data[]` bytes for read exits;
- write MMIO read responses back only for read exits and require an exact response length before mutating the shared `kvm_run` buffer;
- keep raw `kvm_run` pointers and MMIO union layout knowledge inside the vCPU layer;
- add one fixed byte-wide userspace device at guest-physical address `0x2000` that records one-byte writes and returns one configured one-byte read value;
- keep the existing three-argument port-I/O execution API source-compatible while adding one MMIO-aware bounded execution entry point;
- preserve completed-exit budgeting and ordered raw exit-reason tracing across serviceable MMIO exits;
- use a dedicated real-mode fixture with only 4 KiB registered RAM so GPA `0x2000` is deliberately unbacked in that fixture and therefore exits through KVM MMIO rather than slot-0 RAM;
- the deterministic guest must write `W` to GPA `0x2000`, read the same address and receive `R`, emit exactly `RMIO` through the existing debug port, and reach HLT at RIP `0x117`;
- exact execution evidence must prove MMIO write then MMIO read, captured device write `W`, guest-observed readback `R`, four debug-port exits spelling `RMIO`, terminal HLT/RIP/RFLAGS, and continued guest execution after completion of the pending MMIO read;
- stable CI must retain the existing strict long-mode and non-identity ELF64 real-KVM proofs while independently requiring the MMIO proof;
- existing real-mode, port-I/O, long-mode, flat-binary, and ELF64 contracts remain valid.

## Scope boundary

This milestone deliberately does **not** add:

- an MMIO range registry or dynamic device registration;
- multiple MMIO devices or overlapping-range resolution;
- wide/register-bank semantics beyond the fixed one-byte device;
- long-mode virtual-address mapping to an unbacked MMIO GPA;
- PCI configuration space or bus enumeration;
- APIC, interrupt-controller, or interrupt-injection infrastructure;
- eventfd/ioeventfd/irqfd acceleration;
- virtio;
- DMA or IOMMU modeling;
- multiple RAM slots or memory hotplug;
- arbitrary virtual-address windows or caller-defined page-table hierarchy;
- SMP;
- whole-VM snapshots;
- migration;
- resumable execution.

## Promotion rule

After bounded bidirectional MMIO execution is integrated and exact merged-`main` CI is green, perform an architecture/integration audit before selecting another frontier. The leading candidates are a bounded long-mode virtual-MMIO mapping that composes the existing translation layer with the new device path, or a small explicit MMIO range/device-routing layer if the audit shows device composition is the higher-value prerequisite. Interrupt/APIC architecture remains a separate frontier and must not be expanded in parallel with unfinished MMIO work.
