# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `61e407c6daaf9c471d22a79cc600bb7c5e0834fe` through PR #156 (`Replay mutable devices from two restored producers`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; bounded SYSCALL/SYSRET and usercopy; isolated ring3 address spaces; dirty-page tracking; bounded scheduling/wait ownership; versioned single-vCPU/controller/device checkpoints; two-device acceleration reconstruction; transaction-coupled dual-device mutable write/readback replay; acceleration-aware checkpoint quiescence; exact two-vCPU controller ownership including MP states and both LAPICs; one runtime transaction owning both vCPUs, both devices and the fd-free registration pair; a canonical v1 byte format for that complete transaction; and restored multi-producer mutable data-plane replay.

PR #156 sealed the fully quiescent two-producer/two-device cross-layer phase. Its executable proof transports both producer vCPU/controller states plus both queue pages and devices through the canonical #155 wire boundary, restores the same VM, reconstructs two ioeventfd/irqfd registrations, and then has vCPU0 and vCPU1 independently drive only their bound virtio-blk devices through distinct write/readback lifecycles. Each queue finishes at `2/2`, each producer sees exactly two doorbells and two irqfd completions, both mutable backings remain isolated, and the per-producer proofs are `W0aMR0aXD` and `Y1bNZ1bQE`. Exact merged-main commit `61e407c6daaf9c471d22a79cc600bb7c5e0834fe` completed all 56 push-triggered workflows successfully. Do not farm additional quiescent producer/device permutations.

## Selected milestone — serviced completion ownership before irqfd delivery

The next architectural boundary is the first bounded in-flight state that the current device model can describe exactly: KVM_IOEVENTFD has been drained and virtio-blk queue service has completed, so avail/used and backing/status mutations are committed and device ISR is set, but the corresponding irqfd has not yet been signaled. Previously the semantic device could look queue-quiescent while ordinary checkpoint capture had no object owning the missing interrupt delivery.

Implementation continues in PR #157 on `milestone/pending-completion-token` from exact green `main=61e407c6daaf9c471d22a79cc600bb7c5e0834fe`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI, #151–#156 regressions and every applicable permanent hosted-KVM workflow;
- ordinary fully-quiescent virtio-blk and transaction capture must reject ISR-pending state rather than silently accepting a serviced-but-undelivered completion;
- introduce one linear fd-free pending-completion token bound to exactly one BAR, queue 0, and matching nonzero avail/used indices with ISR=queue-interrupt;
- token-aware two-device capture may permit exactly one token-owned pending device while the other device remains fully quiescent;
- host acceleration itself must be quiescent at capture: both ioeventfd counters are drained and no raw fd/eventfd state is serialized;
- version 2 of the existing outer transaction magic must carry the existing canonical checkpoint, registration-pair spec and exactly one pending-completion token; V1 remains the fully-quiescent format;
- V2 encode/decode must be byte-for-byte canonical and reject malformed token length, reserved fields, BAR/index mismatches or a token inconsistent with the nested device checkpoint;
- executable proof must have producer 0 submit a real accelerated sector-0 `T_OUT`; host waits exactly one ioeventfd doorbell, applies the host notification and completes the atomic virtio-blk queue service, but deliberately does not signal irqfd;
- at that boundary first queue indices must be `1/1`, second queue `0/0`, first backing/status mutations complete, first ISR pending, and both acceleration eventfds non-readable;
- ordinary V1 transaction capture must fail closed at that exact boundary; token-aware V2 capture must succeed over the five-page multi-producer ownership set;
- encoder-side ownership must be discarded, V2 decoded/re-encoded canonically, then materialized only from decoded bytes;
- deliberately corrupt all five pages, both vCPU architectural and MP states, PIC/IOAPIC, both LAPICs and both devices; require real mismatch before restore;
- token-aware restore must reproduce the serviced first queue at `1/1`, second queue `0/0`, ISR/token ownership and exact controller/vCPU state before fresh host registrations are reconstructed;
- reconstructed eventfds must start quiescent; the materialized token, not an fd, authorizes exactly one fresh irqfd signal for the restored first completion;
- after that signal, producer 0 must execute its normal interrupt handler, continue into its `T_IN` readback, receive one ordinary second completion and finish with proof `W0aMR0aXD`;
- final queues must be `[[2,2],[0,0]]`, doorbell counts `[2,0]`, irqfd signals `[2,0]`, and producer 0 readback/backing must equal its original distinct write payload;
- producer 1/device 1 remain quiescent throughout, proving the token does not alias completion ownership across devices;
- reconstructed registrations must be deassigned on success and failure;
- add focused token/schema unit coverage, a proof binary, independent KVM integration test and permanent hosted-KVM workflow with compilation outside the 30-second execution timeout.

## Scope boundary

This milestone owns exactly one **serviced but not yet irqfd-signaled** virtio-blk completion. It does not serialize a raw fd, an ioeventfd counter, an irqfd event already consumed by KVM, a partially serviced descriptor chain, two simultaneous pending completions, a third device, concurrent running-vCPU capture, external-storage crash consistency, or cross-host live-migration performance.

## Promotion rule

After the pending-completion token is integrated and exact merged-`main` CI is green, seal this single-token in-flight completion phase. The next architectural audit must distinguish states the current model can still state exactly from states that require a new backend/durability abstraction. Prefer the next bounded ownership token only if queue/interrupt ordering can be represented without guessing kernel or external-storage state; otherwise promote to an explicit storage-backend durability boundary rather than manufacturing migration claims.
