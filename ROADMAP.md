# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe the accumulated design; this roadmap records the currently completed milestone boundary and the explicitly deferred candidate work.

## Current integrated state

The repository has a Phase 73 foundation baseline with typed, owned boundaries for KVM capability discovery, VM/vCPU lifecycle, bounded guest RAM, flat guest loading, configured CPUID/MSR policy and state, vCPU register/special-register/MSR snapshots, centralized VM-exit dispatch, bounded one-vCPU execution, deterministic real-mode fixtures, and the minimal bidirectional debug port-I/O path.

The next bounded milestone adds one additional execution capability without broadening those existing subsystem boundaries:

- one fixed x86-64 long-mode bootstrap contract over the existing single 2 MiB guest-RAM region;
- deterministic four-level page tables using PML4 at `0x1000`, PDPT at `0x2000`, and PD at `0x3000`, with one 2 MiB identity-mapped large page;
- explicit long-mode vCPU state with `CR0.PE|PG`, `CR4.PAE`, `EFER.LME|LMA`, `CR3 = 0x1000`, flat ring-0 64-bit code/data segments, `RIP = 0x10000`, `RSP = 0x1ff000`, and architectural RFLAGS bit 1 set;
- one reviewed 36-byte x86-64 fixture that uses 64-bit instructions, writes the deterministic proof `LM64` through the existing port `0xe9` path, and then executes `HLT`;
- pure regressions for page-table layout, control-register/EFER state, segment state, entry state, and invalid long-mode layout rejection;
- KVM-aware execution regression plus a strict CI command that requires usable `/dev/kvm`, observes `LM64`, and observes the terminal HLT report at RIP `0x10024`;
- CI coverage for formatting, Clippy, tests, build, warning-free rustdoc, declared Rust 1.74 MSRV, and the strict long-mode KVM proof.

## Milestone — x86-64 Long Mode Guest Execution

Correctness contract:

- guest RAM for this bootstrap starts at GPA `0` and is at least 2 MiB;
- virtual addresses `0..0x20_0000` identity-map to the same guest physical addresses through PML4[0] → PDPT[0] → PD[0];
- PML4[0] is `0x2003`, PDPT[0] is `0x3003`, and PD[0] is `0x83`; all other entries in the three bootstrap pages remain zero;
- the page-table pages occupy `0x1000..0x4000`; the entry point and stack pointer may not overlap that reserved bootstrap range;
- the entry must be inside the identity map, and the stack pointer must be non-zero and no greater than the mapped 2 MiB extent;
- `Vcpu::initialize_long_mode` starts from KVM's current special-register state, preserves unrelated bits, enables the required `CR0`, `CR4`, and `EFER` long-mode bits, points `CR3` at the fixed PML4, installs the fixed code/data segment contract, and then writes explicit `RIP`, `RSP`, and `RFLAGS` through `KVM_SET_REGS`;
- the deterministic guest starts at `0x10000`, emits exactly four byte-wide OUT exits containing `L`, `M`, `6`, `4` on debug port `0xe9`, then reaches HLT with RIP `0x10024`;
- successful completion requires the exact proof and terminal HLT; budget exhaustion, malformed layout, unsupported port behavior, KVM entry failure, or another exit is not milestone success;
- the existing real-mode fixtures remain valid and unchanged; the long-mode bootstrap is an additional fixed execution path rather than a replacement loader or general boot protocol.

## Scope boundary

This milestone stops at deterministic x86-64 long-mode guest execution. It does **not** add:

- ELF loading or relocation;
- Linux boot protocol support;
- a general guest virtual-memory/page-table manager beyond the fixed 2 MiB identity map;
- MMIO device modeling;
- APIC, interrupt-controller, or interrupt-injection infrastructure;
- virtio;
- SMP;
- whole-VM snapshots;
- migration;
- resumable execution;
- new unrelated KVM-exit diagnostics or documentation-only cleanup.

## Next-stage candidates — not selected

After this milestone is integrated and exact merged-`main` CI is green, development stops until a later milestone is explicitly selected. Candidate future milestones include ELF loading, a deliberate MMIO/device-model foundation, interrupt/APIC architecture, or a later Linux-boot path. None is selected or authorized by this roadmap entry, and none should be implemented automatically after long-mode completion.
