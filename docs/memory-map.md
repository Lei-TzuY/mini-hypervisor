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

After successful registration, `Vm` owns the `GuestMemory`. The VM descriptor is dropped before the guest mapping, ensuring KVM no longer owns the memory slot when the backing host virtual range is unmapped.

The guest-facing read/write helpers calculate and validate an offset before performing any host pointer arithmetic or copy.

## Scope limit

This document does not define ROM, MMIO, multiple RAM slots, dirty logging, memory hotplug, shared mappings, huge pages, or file-backed guest RAM. Those require explicit future designs rather than implicit extension of slot 0 semantics.
