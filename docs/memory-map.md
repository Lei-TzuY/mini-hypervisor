# Guest memory map

## Current map

The current VM model supports exactly one RAM region. The deterministic lifecycle fixture uses:

| Range | Owner | Purpose |
| --- | --- | --- |
| `0x0000_0000..0x0020_0000` | RAM slot 0 | 2 MiB lifecycle test RAM |

No MMIO ranges exist yet.

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

`GuestMemory` creates a private anonymous read/write host mapping. The VMM registers that mapping as KVM userspace memory slot 0 with no flags. Registration is performed only after all region validation succeeds.

After successful registration, `Vm` owns the `GuestMemory`. `Vm::drop` first submits a zero-sized slot-0 `KVM_SET_USER_MEMORY_REGION` request to remove the registration. Only after confirmed removal is normal mapping destruction allowed. If slot removal fails, the mapping is intentionally leaked rather than leaving a still-live KVM memory slot pointing at unmapped userspace memory.

The guest-facing read/write helpers calculate and validate an offset before performing any host pointer arithmetic or copy.

## Scope limit

This document does not define ROM, MMIO, multiple RAM slots, dirty logging, memory hotplug, shared mappings, huge pages, file-backed guest RAM, or explicit reusable VM shutdown APIs. Those require explicit future designs rather than implicit extension of slot 0 semantics.
