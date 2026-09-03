# Guest memory map

## Current map

The current VM model supports exactly one RAM region plus a fixed high x86 KVM-reserved range:

| Range | Owner | Purpose |
| --- | --- | --- |
| `0x0000_0000..0x0020_0000` | RAM slot 0 | 2 MiB guest RAM |
| `0x0000_1000..0x0000_1001` | flat guest bytes inside RAM | deterministic `HLT` fixture |
| `0x0000_1000..0x0000_101c` | flat guest bytes inside RAM | deterministic CPUID-policy fixture |
| `0x0000_2000..0x0000_2001` | guest result byte inside RAM | debug-port input fixture result |
| `0x0000_2000..0x0000_2008` | guest result words inside RAM | CPUID(1).ECX and KVM-features EAX observations |
| `0xfeff_c000..0xfeff_d000` | KVM x86 reserved | identity-map page |
| `0xfeff_d000..0xff00_0000` | KVM x86 reserved | three-page TSS region |

The fixture rows are not separate KVM memory slots. They document mutually exclusive repository-owned test images and result areas inside slot 0. The high reserved pages are configured through x86 VM ioctls rather than userspace RAM slots. No MMIO ranges exist yet.

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

The guest-facing read/write helpers calculate and validate an offset before performing any host pointer arithmetic or copy.

## Flat guest placement

`FlatGuestImage` validates that its non-empty byte range does not overflow guest physical addressing and that its entry lies inside that byte range. Loading then delegates to `GuestMemory::write`, so image placement must also fit completely inside the configured RAM slot.

The current fixtures are loaded at `0x1000`, which is within the current CS=0 real-mode RIP range and leaves low memory available for result areas and later boot-structure experiments. The CPUID fixture writes two little-endian 32-bit observations to `0x2000` and `0x2004`; host code reads the complete `0x2000..0x2008` range only after the terminal HLT exit.

## Scope limit

This document does not define ROM, MMIO, multiple RAM slots, dirty logging, memory hotplug, shared mappings, huge pages, file-backed guest RAM, ELF program segments, or explicit reusable VM shutdown APIs. Those require explicit future designs rather than implicit extension of slot 0 semantics.
