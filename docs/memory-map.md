# Guest memory map

## Current map

The current VM model supports exactly one RAM region plus a fixed high x86 KVM-reserved range. Repository-owned fixtures are mutually exclusive uses of the same slot 0 RAM:

| Range | Owner | Purpose |
| --- | --- | --- |
| `0x0000_0000..0x0020_0000` | RAM slot 0 | 2 MiB guest RAM and low long-mode identity-mapped backing extent |
| `0x0000_1000..0x0000_2000` | long-mode bootstrap | PML4 page |
| `0x0000_2000..0x0000_3000` | long-mode bootstrap | PDPT page |
| `0x0000_3000..0x0000_4000` | long-mode bootstrap | PD page |
| `0x0000_4000..0x0000_5000` | long-mode bootstrap | bounded alias 4 KiB page table |
| `0x0001_0000..0x0001_0024` | flat guest bytes inside RAM | deterministic 36-byte identity-mapped x86-64 long-mode proof fixture |
| `0x0001_0000..0x0001_0180` | ELF64 physical backing inside RAM | deterministic bounded ELF64 fixture; file-backed prefix plus zeroed BSS tail |
| `0x0040_0000..0x0060_0000` | bounded ELF virtual alias window | optional 4 KiB mappings backed by validated low-RAM pages |
| `0x0040_0100` | ELF64 virtual entry | deterministic non-identity ELF64 fixture entry point |
| `0x001f_f000` | long-mode bootstrap | initial RSP value; stack remains in the low identity map |
| `0x0000_1000..0x0000_1001` | flat guest bytes inside RAM | deterministic real-mode `HLT` fixture |
| `0x0000_1000..0x0000_101c` | flat guest bytes inside RAM | deterministic real-mode CPUID-policy fixture |
| `0x0000_2000..0x0000_2001` | guest result byte inside RAM | debug-port input fixture result |
| `0x0000_2000..0x0000_2008` | guest result words inside RAM | CPUID(1).ECX and KVM-features EAX observations |
| `0xfeff_c000..0xfeff_d000` | KVM x86 reserved | KVM identity-map page |
| `0xfeff_d000..0xff00_0000` | KVM x86 reserved | three-page KVM TSS region |

The fixture rows are not separate KVM memory slots and are not simultaneously active layouts. Legacy real-mode fixtures use low addresses that overlap the long-mode bootstrap page-table pages, while the flat long-mode proof and ELF64 proof are alternative uses of the same low RAM. Each public fixture constructs fresh guest memory and installs only the layout it needs. The high KVM-reserved pages are configured through x86 VM ioctls rather than userspace RAM slots. No MMIO ranges exist.

## Long-mode virtual-address contract

The bootstrap always preserves the existing low identity mapping:

```text
VA 0x0000_0000..0x0020_0000
        │ 2 MiB large-page identity map
        ▼
GPA 0x0000_0000..0x0020_0000
```

The base chain is PML4[0] → PDPT[0] → PD[0]. PML4[0] contains `0x2003`, PDPT[0] contains `0x3003`, and PD[0] contains `0x83`, selecting one present/writable 2 MiB large page. `CR3` points to GPA `0x1000`.

For an identity-only layout, no alias PDE is linked. For a bounded non-identity layout, PD[2] points to the alias page table at GPA `0x4000`; the 512 PTE slots correspond exactly to virtual pages in `0x0040_0000..0x0060_0000`. Each present PTE contains one validated 4 KiB guest-physical backing page plus present/writable flags. Unused PTEs remain zero. The deterministic ELF fixture therefore installs:

```text
VA 0x0040_0000..0x0040_1000
        │ 4 KiB alias PTE
        ▼
GPA 0x0001_0000..0x0001_1000
```

and begins execution at virtual RIP `0x0040_0100` while the instruction bytes reside at GPA `0x0001_0100`.

The physical bootstrap page-table extent is now `0x1000..0x5000`. `LongModeBootLayout::new` retains the identity-only entry contract. `LongModeBootLayout::with_page_mappings` additionally accepts an entry in the fixed alias window only when its 4 KiB virtual page is present in the validated mapping set. Alias virtual pages and physical pages must be 4 KiB aligned; physical backing must remain within the low 2 MiB RAM and outside the bootstrap page-table extent. The stack remains non-zero and inside the low identity map. Both deterministic long-mode fixtures use RSP `0x1ff000`.

This is still not a general guest virtual-memory manager. There is no caller-defined virtual window, page-table allocator, dynamic hierarchy growth, per-page executable/write permission model, NX policy, or page-fault recovery path.

## Address semantics

Guest physical addresses use the `GuestPhysAddr` newtype. A `GuestMemoryRegion` is represented as an aligned base plus a non-zero size with an exclusive end.

Construction rejects:

- zero-sized RAM;
- a base not aligned to 4 KiB;
- a size not aligned to 4 KiB;
- `base + size` overflow.

Access validation rejects:

- accesses beginning below the region;
- non-zero accesses at or beyond the exclusive end;
- accesses crossing the exclusive end;
- `address + length` overflow;
- address/length conversions that cannot fit the host representation.

A zero-length access at the exclusive end is valid.

## Host mapping and KVM registration

`GuestMemory` creates a private anonymous read/write host mapping. The VMM registers that mapping as KVM userspace memory slot 0 with no flags. Registration is performed only after all region validation succeeds and after rejecting overlap with `0xfeff_c000..0xff00_0000`.

After successful registration, `Vm` owns the `GuestMemory`. `Vm::drop` first submits a zero-sized slot-0 `KVM_SET_USER_MEMORY_REGION` request to remove the registration. Only after confirmed removal is normal mapping destruction allowed. If slot removal fails, the mapping is intentionally leaked rather than leaving a still-live KVM memory slot pointing at unmapped userspace memory.

Guest-facing reads/writes, ELF64 segment materialization, and long-mode page-table installation calculate and validate a guest-memory offset before performing any host pointer arithmetic or copy. Virtual ELF addresses are never treated as host pointers or guest-memory offsets. Guest physical addresses are never treated directly as host pointers.

## Guest image placement

`FlatGuestImage` validates that its non-empty byte range does not overflow guest physical addressing and that its entry lies inside that byte range. Loading then delegates to `GuestMemory::write`, so image placement must also fit completely inside the configured RAM slot.

The legacy fixtures remain loaded at `0x1000`. The CPUID fixture writes two little-endian 32-bit observations to `0x2000` and `0x2004`; host code reads the complete `0x2000..0x2008` range only after terminal HLT.

The flat long-mode fixture is loaded at GPA/VA `0x10000`, outside the reserved bootstrap page-table pages. It emits proof through port I/O rather than using a guest-memory result buffer and reaches terminal HLT at RIP `0x10024`.

`Elf64GuestImage` validates virtual and physical `PT_LOAD` semantics separately. A low virtual range in the identity window requires `p_vaddr == p_paddr`. A non-identity range must fit completely inside `0x400000..0x600000`, keep the same 4 KiB byte offset between `p_vaddr` and `p_paddr`, and use physical backing wholly inside the low 2 MiB RAM outside `0x1000..0x5000`. Load segments may not overlap in either virtual or physical address space. File bytes and BSS zeroing target only validated physical backing, while the vCPU entry is the validated virtual ELF entry.

The deterministic ELF fixture has one executable `PT_LOAD` with virtual base `0x400000`, physical base `0x10000`, virtual entry `0x400100`, and memory size `0x180`. Its validated mapping plan installs the first alias PTE from virtual page `0x400000` to physical page `0x10000`; execution emits `LM64` and reaches terminal HLT at virtual RIP `0x400124`.

## Scope limit

This document does not define ROM, MMIO, multiple RAM slots, dirty logging, memory hotplug, shared mappings, file-backed guest RAM, a reusable huge-page allocator, arbitrary virtual windows, dynamic page-table allocation, per-page permission policy, ELF relocations or load bias, `ET_DYN`/PIE, dynamic linking, Linux boot structures, or explicit reusable VM shutdown APIs. Those require explicit future milestone designs rather than implicit extension of slot 0 or the bounded alias contract.
