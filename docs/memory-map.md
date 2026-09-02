# Guest memory map

## Current map

The current VM model supports exactly one RAM region. The deterministic lifecycle and HLT fixtures use:

| Range | Owner | Purpose |
| --- | --- | --- |
| `0x0000_0000..0x0020_0000` | RAM slot 0 | 2 MiB guest RAM |
| `0x0000_1000..0x0000_1001` | flat guest bytes inside RAM | deterministic `HLT` fixture |

The second row is not a separate KVM memory slot; it documents the byte currently occupied by the test guest inside slot 0. No MMIO ranges exist yet.

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

## Flat guest placement

`FlatGuestImage` validates that its non-empty byte range does not overflow guest physical addressing and that its entry lies inside that byte range. Loading then delegates to `GuestMemory::write`, so image placement must also fit completely inside the configured RAM slot.

The current HLT fixture is loaded at `0x1000`, which is within the current CS=0 real-mode RIP range and leaves low memory available for later boot-structure experiments.

## Scope limit

This document does not define ROM, MMIO, multiple RAM slots, dirty logging, memory hotplug, shared mappings, huge pages, file-backed guest RAM, ELF program segments, or explicit reusable VM shutdown APIs. Those require explicit future designs rather than implicit extension of slot 0 semantics.
