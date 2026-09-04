# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the currently integrated capability boundary and the selected next executable milestone.

## Current integrated state

The Phase 73 foundation and the x86-64 long-mode execution milestone are integrated on `main`.

The repository now has typed, owned boundaries for KVM capability discovery, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, configured CPUID/MSR policy and state, vCPU register/special-register/MSR snapshots, centralized VM-exit dispatch, bounded one-vCPU execution, deterministic real-mode fixtures, bidirectional debug port I/O, and one strict x86-64 long-mode bootstrap/execution path.

The integrated long-mode path uses a fixed 2 MiB guest-RAM identity map with PML4 at `0x1000`, PDPT at `0x2000`, PD at `0x3000`, explicit CR0/CR3/CR4/EFER and segment/register state, and a real-KVM proof that emits `LM64` then reaches HLT. Merged-main CI requires that proof rather than treating `/dev/kvm` execution as optional evidence.

## Selected milestone — bounded ELF64 executable loading and execution

This milestone promotes the long-mode path from reviewed flat machine-code bytes to one validated standard executable artifact without introducing a general operating-system boot stack.

Acceptance contract:

- accept ELF64, little-endian, x86-64 `ET_EXEC` images only;
- validate the fixed ELF64 header and the complete program-header table before deriving slices;
- require at least one non-empty `PT_LOAD` segment;
- require `p_filesz <= p_memsz` and checked file offsets, file extents, guest-address extents, and host-size conversions;
- accept segment alignment `0`, `1`, or a power of two, and for aligned segments require `p_offset` and `p_vaddr` congruence;
- require `p_vaddr == p_paddr` for this milestone because execution still uses the integrated fixed identity map rather than a general guest virtual-memory manager;
- require every loadable segment to remain within the 2 MiB identity-mapped extent;
- reject loadable segments that overlap the reserved bootstrap page tables at `0x1000..0x4000` or overlap another loadable segment;
- require the ELF entry point to lie inside an executable, file-backed `PT_LOAD` range;
- materialize each file-backed range into guest RAM and explicitly zero `p_memsz - p_filesz` bytes for BSS semantics;
- construct `LongModeBootLayout` from the validated ELF entry, then enter the existing x86-64 execution path without a second long-mode policy;
- the deterministic milestone fixture must parse as ELF64, load through the production loader, emit exactly `LM64` through four byte-wide OUT exits on port `0xe9`, and reach HLT at RIP `0x10124`;
- stable CI must retain the existing strict long-mode proof and add an independent strict real-KVM ELF64 execution proof;
- the existing flat-binary fixtures remain supported and unchanged.

## Scope boundary

This milestone deliberately does **not** add:

- ELF relocations;
- `ET_DYN`, PIE, or a load-bias allocator;
- dynamic linking or an ELF interpreter;
- symbol loading, debugging information, or section-header semantics;
- arbitrary guest virtual mappings or a general page-table manager;
- Linux boot protocol support;
- MMIO device modeling;
- APIC, interrupt-controller, or interrupt-injection infrastructure;
- virtio;
- SMP;
- whole-VM snapshots;
- migration;
- resumable execution.

## Promotion rule

After bounded ELF64 execution is integrated and exact merged-`main` CI is green, perform an architecture/integration audit before selecting another frontier. Highest-value candidates are a deliberate MMIO/device-model foundation, a more general guest virtual-memory/layout layer needed for non-identity ELF mappings, or interrupt/APIC architecture. Do not expand several of these surfaces in parallel.
