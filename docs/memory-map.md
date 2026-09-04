# Guest memory map

## Current map

The current VM model supports exactly one RAM region plus a fixed high x86 KVM-reserved range. Repository-owned fixtures are mutually exclusive uses of the same slot 0 RAM:

| Range | Owner | Purpose |
| --- | --- | --- |
| `0x0000_0000..0x0020_0000` | RAM slot 0 | 2 MiB guest RAM and long-mode identity-mapped extent |
| `0x0000_1000..0x0000_2000` | long-mode bootstrap | PML4 page |
| `0x0000_2000..0x0000_3000` | long-mode bootstrap | PDPT page |
| `0x0000_3000..0x0000_4000` | long-mode bootstrap | PD page |
| `0x0001_0000..0x0001_0024` | flat guest bytes inside RAM | deterministic 36-byte x86-64 long-mode proof fixture |
| `0x001f_f000` | long-mode bootstrap | initial RSP value; stack grows downward if used |
| `0x0000_1000..0x0000_1001` | flat guest bytes inside RAM | deterministic real-mode `HLT` fixture |
| `0x0000_1000..0x0000_101c` | flat guest bytes inside RAM | deterministic real-mode CPUID-policy fixture |
| `0x0000_2000..0x0000_2001` | guest result byte inside RAM | debug-port input fixture result |
| `0x0000_2000..0x0000_2008` | guest result words inside RAM | CPUID(1).ECX and KVM-features EAX observations |
| `0xfeff_c000..0xfeff_d000` | KVM x86 reserved | KVM identity-map page |
| `0xfeff_d000..0xff00_0000` | KVM x86 reserved | three-page KVM TSS region |

The fixture rows are not separate KVM memory slots and are not simultaneously active layouts. In particular, the legacy real-mode fixtures use low addresses that overlap the long-mode bootstrap page-table pages; each public fixture constructs fresh guest memory and installs only the layout it needs. The high KVM-reserved pages are configured through x86 VM ioctls rather than userspace RAM slots. No MMIO ranges exist.

## Long-mode virtual-address contract

The x86-64 milestone defines one fixed translation only:

```text
VA 0x0000_0000..0x0020_0000
        │ identity map
        ▼
GPA 0x0000_0000..0x0020_0000
```

The chain is PML4[0] → PDPT[0] → PD[0]. PML4[0] contains `0x2003`, PDPT[0] contains `0x3003`, and PD[0] contains `0x83`, selecting one present/writable 2 MiB large page. All other entries in the three bootstrap pages are zero. `CR3` points to GPA `0x1000`.

`LongModeBootLayout` requires RAM base 0 and at least 2 MiB, rejects an entry outside the identity map or inside `0x1000..0x4000`, and rejects a zero/out-of-map stack pointer or one overlapping the bootstrap page-table range. The deterministic fixture uses entry `0x10000` and RSP `0x1ff000`.

This is not a general guest virtual-memory manager. There is no arbitrary VA→GPA mapping API, page-table allocator, permission policy surface, or dynamic page-table growth in this milestone.

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

Guest-facing reads/writes and long-mode page-table installation calculate and validate a guest-memory offset before performing any host pointer arithmetic or copy. Guest physical addresses are never treated directly as host pointers.

## Flat guest placement

`FlatGuestImage` validates that its non-empty byte range does not overflow guest physical addressing and that its entry lies inside that byte range. Loading then delegates to `GuestMemory::write`, so image placement must also fit completely inside the configured RAM slot.

The legacy fixtures remain loaded at `0x1000`. The CPUID fixture writes two little-endian 32-bit observations to `0x2000` and `0x2004`; host code reads the complete `0x2000..0x2008` range only after terminal HLT.

The long-mode fixture is loaded at `0x10000`, outside the reserved bootstrap page-table pages. It emits proof through port I/O rather than using a guest-memory result buffer and reaches terminal HLT at RIP `0x10024`.

## Scope limit

This document does not define ROM, MMIO, multiple RAM slots, dirty logging, memory hotplug, shared mappings, file-backed guest RAM, a reusable huge-page allocator, arbitrary virtual mappings, ELF program segments, Linux boot structures, or explicit reusable VM shutdown APIs. Those require explicit future milestone designs rather than implicit extension of slot 0 or the fixed identity-map semantics.
