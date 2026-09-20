# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `57693edbed9489a48e8d3e591136a09e77739509` through PR #157 (`Own serviced virtio-blk completions until irqfd delivery`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; bounded SYSCALL/SYSRET and usercopy; isolated ring3 address spaces; dirty-page tracking; bounded scheduling/wait ownership; versioned single-vCPU/controller/device checkpoints; two-device acceleration reconstruction; transaction-coupled dual-device mutable write/readback replay; acceleration-aware checkpoint quiescence; exact two-vCPU controller ownership including MP states and both LAPICs; one runtime transaction owning both vCPUs, both devices and the fd-free registration pair; a canonical fully-quiescent v1 byte format; restored multi-producer mutable data-plane replay; and a v2 token that owns one serviced virtio-blk completion until irqfd delivery.

PR #157 sealed the first bounded in-flight completion phase. Its executable proof lets producer 0 submit a real accelerated sector-0 write, drains the ioeventfd and fully services the queue, but deliberately withholds irqfd delivery. At that boundary queue 0 is `1/1`, backing/status/used-ring mutation is committed, ISR is pending, both host eventfds are quiescent, ordinary v1 capture fails closed, and v2 captures one linear fd-free completion token. The encoded owner is discarded, decoded/re-encoded canonically, exact state is restored, fresh host registrations are reconstructed, and only the materialized token authorizes the missing irqfd signal before the producer completes its readback. Exact merged-main commit `57693edbed9489a48e8d3e591136a09e77739509` completed all 57 push-triggered workflows successfully. Do not farm additional serviced-completion BAR permutations.

## Selected milestone — drained notification ownership before queue service

The next bounded in-flight state is earlier in the same lifecycle and is still representable exactly by the current model: KVM_IOEVENTFD has consumed the guest MMIO notify and userspace has drained that eventfd count, then the host notification has been materialized into the semantic virtio-blk device as `notify_pending=true`, but queue service has not begun. No raw fd remains pending, the queue's descriptor/data/status bytes live in the already-owned guest pages, queue indices and backing are still unchanged, and ISR remains clear.

Implementation continues on `milestone/pending-notification-token` from exact green `main=57693edbed9489a48e8d3e591136a09e77739509`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, ordinary CI, #151–#157 regressions and every applicable permanent hosted-KVM workflow;
- ordinary fully-quiescent virtio-blk and transaction capture must continue to reject `notify_pending=true`;
- introduce exactly one linear fd-free pending-notification token bound to a BAR, queue 0 and the current equal `last_avail_idx/last_used_idx`; no raw eventfd state may be serialized;
- token capture is valid only when `notify_pending=true`, ISR is zero and the device has not advanced queue indices since the prior completion boundary;
- the other device must remain fully quiescent, and reconstructed/live devices must be fully quiescent before token-aware restore;
- token-aware semantic materialization must restore `notify_pending=true` only for the bound device and must fail closed if BAR/queue/index/ISR binding is inconsistent;
- version 3 of the existing outer transaction magic must carry the existing canonical two-vCPU/two-device checkpoint, registration-pair spec and exactly one pending-notification token; v1 remains fully quiescent and v2 remains serviced-completion ownership;
- v3 encode/decode must be canonical and reject malformed token length, reserved fields, BAR/index mismatch or token/state inconsistency;
- executable proof must have producer 0 submit a real accelerated sector-0 `T_OUT`; host waits exactly one ioeventfd doorbell, drains it, applies the host notification, and deliberately stops before queue processing;
- at that capture boundary both host eventfds must be non-readable, both device queue indices remain `0/0`, first backing remains the deterministic pre-write value, first ISR is clear, and only first device reports token-owned pending notification;
- ordinary transaction capture must fail closed at that exact state; token-aware v3 capture must succeed over the existing five-page multi-producer ownership set;
- encoder-side ownership must be discarded, v3 decoded/re-encoded canonically, then materialized only from decoded bytes;
- deliberately corrupt all five pages, both architectural/MP states, PIC/IOAPIC, both LAPICs and both devices; require real mismatch before restore;
- token-aware restore must reproduce both queues at `0/0`, the pending semantic notification, exact controller/vCPU/device state, and fresh host registrations whose eventfds are quiescent;
- queue service after restore must consume the materialized semantic notification without requiring a second write-doorbell event, mutate first backing and used ring exactly once, then one reconstructed irqfd signal must deliver that completion;
- producer 0 must continue into its ordinary `T_IN` readback, receive the second normal accelerated completion and finish with proof `W0aMR0aXD`;
- final queues must be `[[2,2],[0,0]]`, total doorbell counts `[2,0]` (one pre-capture write notify plus one post-restore read notify), irqfd signals `[2,0]`, and final readback/backing must equal producer 0's original payload;
- producer 1/device 1 remain quiescent throughout; reconstructed registrations must be deassigned on success and failure;
- add focused token/schema unit coverage, proof binary, independent KVM integration test and permanent hosted-KVM workflow with compilation outside the 30-second execution timeout.

## Scope boundary

This milestone owns exactly one **drained and semantically materialized notification before queue service**. It does not serialize an eventfd counter, raw fd, partially serviced descriptor chain, already-signaled irqfd event, two simultaneous pending notifications, a third device, concurrent running-vCPU capture, external-storage durability or cross-host live migration.

The current virtio-blk backing is still a bounded in-memory four-sector array. Therefore this milestone makes no persistence, crash-consistency, fsync or external-storage durability claim.

## Promotion rule

After pending-notification ownership is integrated and exact merged-`main` CI is green, seal the in-memory notification/completion ownership chain. The next architecture audit should not farm more token permutations. Because queue service is atomic in the current implementation, there is no honest partially-serviced descriptor state to migrate. Promote instead to an explicit storage-backend boundary if the project is ready to add a real backend abstraction and externally verifiable persistence semantics; otherwise choose another cross-layer capability with concrete implementation evidence rather than manufacturing durability claims.
