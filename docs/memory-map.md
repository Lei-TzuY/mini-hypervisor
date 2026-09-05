# Guest memory map

## Current map

The current VM model supports exactly one RAM region plus a fixed high x86 KVM-reserved range. Repository-owned fixtures are mutually exclusive uses of slot 0 RAM or deliberately unbacked device addresses:

| Range | Owner | Purpose |
| --- | --- | --- |
| `0x0000_0000..0x0020_0000` | RAM slot 0 | 2 MiB guest RAM and low long-mode identity-mapped backing extent used by long-mode, ELF, and long-mode-MMIO fixtures |
| `0x0000_0000..0x0000_1000` | real-mode MMIO fixture RAM slot 0 | dedicated 4 KiB RAM layout; mutually exclusive with the 2 MiB layouts above |
| GPA `0x0000_2000` | real-mode MMIO byte device in the 4 KiB fixture only | intentionally outside that fixture's registered RAM so KVM exits to userspace for one-byte read/write access |
| `0x0000_1000..0x0000_2000` | long-mode bootstrap | PML4 page |
| `0x0000_2000..0x0000_3000` | long-mode bootstrap | PDPT page |
| `0x0000_3000..0x0000_4000` | long-mode bootstrap | PD page |
| `0x0000_4000..0x0000_5000` | long-mode bootstrap | bounded alias 4 KiB page table |
| `0x0001_0000..0x0001_0024` | flat guest bytes inside RAM | deterministic 36-byte identity-mapped x86-64 long-mode proof fixture |
| `0x0001_0000..0x0001_001e` | flat guest bytes inside RAM | deterministic 30-byte x86-64 virtual-MMIO proof fixture |
| `0x0001_0000..0x0001_0180` | ELF64 physical backing inside RAM | deterministic bounded ELF64 fixture; file-backed prefix plus zeroed BSS tail |
| VA `0x0040_0000..0x0060_0000` | bounded alias window | fixed 512-page virtual window shared by validated RAM-backed ELF aliases and the bounded virtual-MMIO fixture |
| VA `0x0040_0100` | ELF64 virtual entry | deterministic non-identity ELF64 fixture entry point |
| VA `0x0050_0000..0x0050_1000` | long-mode virtual MMIO mapping | one explicit device PTE in the fixed alias PT |
| GPA `0x1000_0000..0x1000_1000` | long-mode MMIO byte-device page | deliberately outside the 2 MiB slot-0 RAM and reached only through the fixed VA `0x500000` mapping |
| `0x001f_f000` | long-mode bootstrap | initial RSP value; stack remains in the low identity map |
| `0x0000_0100..0x0000_0117` | flat guest bytes inside real-mode MMIO fixture RAM | deterministic bidirectional MMIO proof fixture |
| `0x0000_1000..0x0000_1001` | flat guest bytes inside RAM | deterministic real-mode `HLT` fixture |
| `0x0000_1000..0x0000_101c` | flat guest bytes inside RAM | deterministic real-mode CPUID-policy fixture |
| `0x0000_2000..0x0000_2001` | guest result byte inside RAM | debug-port input fixture result |
| `0x0000_2000..0x0000_2008` | guest result words inside RAM | CPUID(1).ECX and KVM-features EAX observations |
| `0xfeff_c000..0xfeff_d000` | KVM x86 reserved | KVM identity-map page |
| `0xfeff_d000..0xff00_0000` | KVM x86 reserved | three-page KVM TSS region |

The fixture rows are not simultaneous KVM memory slots. Each public fixture constructs fresh guest memory and installs only the layout it needs. This distinction is essential for GPA `0x2000`: in long-mode layouts it is RAM containing the PDPT page; in debug-port input/CPUID fixtures it is RAM used for guest result bytes; only the dedicated 4 KiB real-mode MMIO fixture leaves it unbacked. Likewise GPA `0x10000000` is not a second RAM slot: the long-mode virtual-MMIO layout explicitly requires it to remain outside the configured slot-0 RAM and maps one guest virtual page to it so KVM reports accesses as `KVM_EXIT_MMIO`.

## Real-mode MMIO fixture contract

The original bounded MMIO proof intentionally avoids introducing another RAM slot or a hole inside one registered slot. It registers exactly one 4 KiB slot-0 region:

```text
GPA 0x0000_0000..0x0000_1000   registered RAM
GPA 0x0000_2000                 fixed userspace MMIO byte device
```

The reviewed 23-byte real-mode guest starts at RIP `0x100`. It writes byte `W` to absolute address `0x2000`, reads one byte from the same address, receives configured byte `R` from userspace, writes `R`, `M`, `I`, `O` through port `0xe9`, and halts at RIP `0x117`. Because `0x2000` is outside the only registered region in this fixture, the two memory accesses are evidence of KVM's userspace MMIO path rather than normal guest RAM access.

This remains a fixture-specific physical device address, not a globally reserved GPA across every VM layout.

## Long-mode virtual-address contract

The bootstrap always preserves the existing low identity mapping:

```text
VA 0x0000_0000..0x0020_0000
        │ 2 MiB large-page identity map
        ▼
GPA 0x0000_0000..0x0020_0000
```

The base chain is PML4[0] → PDPT[0] → PD[0]. PML4[0] contains `0x2003`, PDPT[0] contains `0x3003`, and PD[0] contains `0x83`, selecting one present/writable 2 MiB large page. `CR3` points to GPA `0x1000`.

For an identity-only `LongModeBootLayout`, no alias PDE is linked. For a bounded RAM-backed non-identity layout, PD[2] points to the alias page table at GPA `0x4000`; the 512 PTE slots correspond exactly to virtual pages in `0x0040_0000..0x0060_0000`. `LongModePageMapping` remains a RAM-only contract: each present PTE created by `LongModeBootLayout::with_page_mappings` contains one validated 4 KiB physical page inside low guest RAM, outside bootstrap page-table pages, plus present/writable flags. Unused PTEs remain zero. The deterministic ELF fixture installs:

```text
VA 0x0040_0000..0x0040_1000
        │ validated RAM-backed 4 KiB alias PTE
        ▼
GPA 0x0001_0000..0x0001_1000
```

and begins execution at virtual RIP `0x0040_0100` while the instruction bytes reside at GPA `0x0001_0100`.

The long-mode virtual-MMIO fixture deliberately uses a separate `LongModeMmioBootLayout` rather than relaxing `LongModePageMapping`. It first installs the same identity-only bootstrap, then links PD[2] to the existing alias PT and writes exactly PTE index 256 for virtual page `0x500000`:

```text
VA 0x0050_0000..0x0050_1000
        │ explicit device PTE
        ▼
GPA 0x1000_0000..0x1000_1000   unbacked device page
```

The device GPA is 4 KiB aligned and the public layout constructor rejects any RAM region whose exclusive end extends beyond `0x10000000`, because that would turn the fixed device address into normal slot-0 RAM. The current deterministic fixture uses 2 MiB RAM, so the device page is far outside registered memory. The guest code and stack remain in the low identity mapping; only the data access at VA `0x500000` uses the device PTE.

The physical bootstrap page-table extent remains `0x1000..0x5000`. `LongModeBootLayout::new` retains the identity-only entry contract. `LongModeBootLayout::with_page_mappings` accepts an entry in the fixed alias window only when its 4 KiB virtual page is present in the validated RAM-backed mapping set. `LongModeMmioBootLayout` does not change those rules and does not use the device VA as an execution entry. The stack remains non-zero and inside the low identity map. The deterministic long-mode, ELF64, and virtual-MMIO fixtures use RSP `0x1ff000`.

This is still not a general guest virtual-memory manager. There is no caller-defined virtual window, page-table allocator, dynamic hierarchy growth, per-page executable/write permission model, NX policy, arbitrary virtual-MMIO mapping, or page-fault recovery path.

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

Guest-facing reads/writes, ELF64 segment materialization, and long-mode page-table installation calculate and validate a guest-memory offset before performing any host pointer arithmetic or copy. Virtual ELF addresses are never treated as host pointers or guest-memory offsets. Guest physical addresses are never treated directly as host pointers. MMIO device accesses are not routed through `GuestMemory`: KVM reports either the real-mode fixture's unbacked GPA `0x2000` or the long-mode fixture's translated unbacked GPA `0x10000000` through the same typed MMIO exit path.

## Guest image placement

`FlatGuestImage` validates that its non-empty byte range does not overflow guest physical addressing and that its entry lies inside that byte range. Loading then delegates to `GuestMemory::write`, so image placement must also fit completely inside the configured RAM slot.

The legacy fixtures remain loaded at `0x1000`. The CPUID fixture writes two little-endian 32-bit observations to `0x2000` and `0x2004`; host code reads the complete `0x2000..0x2008` range only after terminal HLT.

The real-mode MMIO fixture is the deliberate exception to the legacy `0x1000` entry convention: its 23-byte image resides at `0x100..0x117` inside its dedicated 4 KiB RAM region so that absolute address `0x2000` remains outside registered memory.

The flat long-mode fixture is loaded at GPA/VA `0x10000`, outside the reserved bootstrap page-table pages. It emits proof through port I/O rather than using a guest-memory result buffer and reaches terminal HLT at RIP `0x10024`.

The long-mode virtual-MMIO fixture also loads executable bytes identity-mapped at GPA/VA `0x10000`. Its 30-byte 64-bit guest materializes VA `0x500000` in RBX, writes `W` through that virtual address, reads configured byte `R` back through the same mapping, emits `R64M` through port `0xe9`, and reaches HLT at RIP `0x1001e`. KVM reports both memory exits at translated GPA `0x10000000`, proving that the page-table mapping and userspace MMIO path compose.

`Elf64GuestImage` validates virtual and physical `PT_LOAD` semantics separately. A low virtual range in the identity window requires `p_vaddr == p_paddr`. A non-identity range must fit completely inside `0x400000..0x600000`, keep the same 4 KiB byte offset between `p_vaddr` and `p_paddr`, and use physical backing wholly inside the low 2 MiB RAM outside `0x1000..0x5000`. Load segments may not overlap in either virtual or physical address space. File bytes and BSS zeroing target only validated physical backing, while the vCPU entry is the validated virtual ELF entry.

The deterministic ELF fixture has one executable `PT_LOAD` with virtual base `0x400000`, physical base `0x10000`, virtual entry `0x400100`, and memory size `0x180`. Its validated mapping plan installs the first alias PTE from virtual page `0x400000` to physical page `0x10000`; execution emits `LM64` and reaches terminal HLT at virtual RIP `0x400124`.

## Scope limit

This document does not define multiple RAM slots, dirty logging, memory hotplug, shared mappings, file-backed guest RAM, a reusable huge-page allocator, arbitrary virtual windows, dynamic page-table allocation, per-page permission policy, caller-defined virtual-MMIO mappings, MMIO range/device registration, PCI layout, ELF relocations or load bias, `ET_DYN`/PIE, dynamic linking, Linux boot structures, or explicit reusable VM shutdown APIs. The integrated virtual-MMIO composition is exactly one fixed device PTE and one fixed byte device; broader mapping or routing semantics require separately selected milestones.
