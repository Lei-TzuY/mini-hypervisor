# Roadmap

This file is the authoritative live roadmap for bounded implementation slices. Architecture and safety documents describe accumulated design; this roadmap records the integrated capability boundary and selected next executable milestone.

## Current integrated state

`main` is `c48df7e785f484b158a30fe462d976991792f339` through PR #159 (`Add synced file-backed virtio-blk storage`). The repository integrates the Phase 73 foundation; x86-64 and bounded ELF64 execution; userspace MMIO and controller-backed interrupts; direct, irqfd and eventfd asynchronous delivery; PCI/virtio execution; bounded SMP/IPI/timer/TLB-shootdown behavior; ring3/TSS transitions; bounded SYSCALL/SYSRET and usercopy; isolated ring3 address spaces; dirty-page tracking; bounded scheduling/wait ownership; versioned single- and two-vCPU checkpoint transactions; transaction-coupled multi-producer virtio-blk replay; fd-free pending notification/completion ownership; and a bounded file-backed virtio-blk mode with synchronized writes and drop/reopen readback.

PR #159 sealed the basic external-storage persistence boundary. Its executable proof performs a real sector-0 `T_OUT`, calls `sync_all` before guest completion becomes visible, inspects the raw host file, drops the device, opens a fresh device from the same file and performs a real `T_IN` readback. Current checkpoint schemas continue to fail closed for external storage identity. Exact merged-main commit `c48df7e785f484b158a30fe462d976991792f339` completed all 58 push-triggered workflows successfully. Do not farm more path/reopen variants.

## Selected milestone — pin runtime file-backend identity

The next correctness boundary is more fundamental than checkpoint serialization. The file-backed device currently remembers only a `PathBuf` and reopens that pathname for every guest write. A pathname is a lookup instruction, not a stable storage identity: after rename/unlink/replacement, a later `open(path)` may resolve to a different host file than the one whose bytes were loaded when the device was created.

Implementation continues on `milestone/pinned-file-backend-identity` from exact green `main=c48df7e785f484b158a30fe462d976991792f339`.

Acceptance contract:

- preserve Rust 1.74 shipped-target MSRV, all in-memory/file-backed virtio-blk regressions, checkpoint fail-closed behavior and permanent workflows;
- retain the already-open host file object for the lifetime of a file-backed device instead of reopening its origin pathname on each `T_OUT`;
- record the same-host runtime identity as Linux `st_dev + st_ino`; this identity is diagnostic/runtime evidence only and is not a cross-host or cryptographic identifier;
- cloning a runtime file-backed device must retain the same pinned file object/identity rather than resolving the pathname again;
- guest write persistence must use offset writes on the pinned handle followed by `sync_all` before status/used-ring/ISR completion becomes visible;
- preserve the #159 invariant that real host backing write failure leaves queue indices, ISR, guest status/used ring and in-memory cache uncommitted;
- executable proof must create a file-backed device, record its open-file identity, rename the original pathname, create a different valid backing at the original pathname, then submit a real sector-0 `T_OUT` through the normal atomic queue path;
- the renamed original inode must receive and synchronize the guest payload while the replacement pathname file remains at deterministic initial contents;
- the replacement file must have a distinct `st_dev/st_ino` identity from the pinned device and the device's identity must remain unchanged;
- existing checkpoint capture must still reject the pinned external backend because raw file handles and host-local inode identity remain outside current checkpoint schemas;
- add focused unit coverage, independent integration test, proof binary and permanent workflow.

## Scope boundary

This milestone pins one normal host-file object on Linux. It does not serialize file descriptors, pathname strings, inode numbers or mount identity into checkpoint bytes; does not claim identity survives host reboot/cross-host migration; does not add concurrent external-writer coherence, file locking, direct I/O, crash consistency or production storage performance.

Path rename/replacement is used only as executable evidence that runtime I/O is attached to the opened file object rather than repeated pathname lookup.

## Promotion rule

After pinned runtime identity is integrated and exact merged-`main` CI is green, seal the runtime file-identity phase. Only then evaluate external-storage checkpoint coordination. A future checkpoint binding must use an explicit host-supplied storage identity/rebind contract and verify the rebound object before materialization; it must not serialize raw fds or silently trust a pathname. If that contract cannot be made explicit and testable, promote instead to bounded storage failure/recovery semantics rather than inventing migration claims.
