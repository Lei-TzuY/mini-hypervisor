# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the currently integrated capability boundary and the selected next executable milestone.

## Current integrated state

The Phase 73 foundation, deterministic x86-64 long-mode execution, and bounded ELF64 `ET_EXEC` loading/execution are integrated on `main`.

The repository has typed, owned boundaries for KVM capability discovery, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, configured CPUID/MSR policy and state, vCPU register/special-register/MSR snapshots, centralized VM-exit dispatch, bounded one-vCPU execution, deterministic real-mode fixtures, bidirectional debug port I/O, one strict x86-64 long-mode bootstrap/execution path, and one bounded ELF64 loader that validates and materializes `PT_LOAD` segments including explicit BSS zeroing.

Merged-main CI requires real-KVM evidence for both the flat x86-64 long-mode proof and the ELF64 proof rather than treating `/dev/kvm` execution as optional evidence.

## Selected milestone — bounded non-identity ELF64 virtual mapping

This milestone promotes the integrated ELF64 path beyond `p_vaddr == p_paddr` while remaining deliberately smaller than a general guest virtual-memory manager.

Acceptance contract:

- preserve the existing low `0..0x20_0000` 2 MiB identity mapping and its strict flat long-mode proof at RIP `0x10024`;
- reserve a fixed ELF alias virtual window `0x40_0000..0x60_0000`;
- keep PML4 at GPA `0x1000`, PDPT at `0x2000`, and PD at `0x3000`, and reserve a fourth bootstrap page at GPA `0x4000` for the alias page table;
- extend the bootstrap-reserved physical extent to `0x1000..0x5000`;
- represent non-identity mappings as validated 4 KiB virtual-page to guest-physical-page pairs;
- keep every alias physical backing page inside the existing low 2 MiB RAM and outside the bootstrap page-table extent;
- allow ELF `PT_LOAD` `p_vaddr != p_paddr` only when the complete virtual range is inside the fixed alias window;
- require virtual and physical page offsets to match for an aliased segment so 4 KiB PTE materialization preserves byte offsets;
- continue accepting identity-mapped ELF load segments in the low 2 MiB window only when virtual and physical addresses are equal;
- validate ELF virtual extents and physical backing extents independently, including overflow, RAM bounds, bootstrap overlap, virtual overlap, physical overlap, and conflicting alias PTEs;
- require the ELF entry point to lie inside an executable file-backed `PT_LOAD` range and require an aliased entry page to be present in the validated mapping plan;
- load file bytes and zero BSS only into the validated physical backing while programming vCPU RIP with the validated virtual ELF entry;
- the deterministic ELF fixture must map virtual `0x40_0000` to physical `0x1_0000`, enter at virtual `0x40_0100`, emit exactly `LM64` through four byte-wide OUT exits on port `0xe9`, and reach HLT at virtual RIP `0x40_0124`;
- stable CI must keep the existing strict flat long-mode real-KVM proof green while independently requiring the non-identity ELF proof;
- existing real-mode, flat-binary, and identity-mapped ELF contracts remain valid.

## Scope boundary

This milestone deliberately does **not** add:

- arbitrary virtual-address windows or caller-defined page-table hierarchy;
- dynamic page-table allocation or growth;
- per-segment executable/writable page permissions or NX policy;
- `ET_DYN`, PIE, relocations, or a load-bias allocator;
- dynamic linking or an ELF interpreter;
- Linux boot protocol support;
- MMIO device modeling;
- APIC, interrupt-controller, or interrupt-injection infrastructure;
- virtio;
- SMP;
- whole-VM snapshots;
- migration;
- resumable execution.

## Promotion rule

After bounded non-identity ELF execution is integrated and exact merged-`main` CI is green, perform an architecture/integration audit before selecting another frontier. The leading next candidate is a deliberate MMIO/device-model foundation because the project will then have an explicit virtual-to-guest-physical mapping layer to build on; interrupt/APIC architecture and a more general virtual-memory/page-permission layer remain separate candidates. Do not expand several of these surfaces in parallel.
