# Native input and network hand-off

Implemented on dev after v0.9.3. The design reference is lowlat revision
`7b4dc4ae79990ea8c59a108c80b3c89b695c3cb4`, particularly
[its event-driven IO shell](https://github.com/nomi-san/lowlat/blob/7b4dc4ae79990ea8c59a108c80b3c89b695c3cb4/docs/02-io-shell.md).
This adopts bounded reusable storage and shorter critical sections, not BUD's
wire protocol, retransmission timers, congestion algorithm or socket presets.
No lowlat source is copied into the client.

## Input

The producer queue retains its 256-event limit and FIFO behavior. Its storage is
reserved at construction; a second bounded deque is allocated on the consumer's
first batch. Successive batches swap these two allocations under the mutex, then
dispatch outside it. The queue no longer allocates a fresh batch vector or moves
each event into that vector while blocking the producer. Notifications are sent
after releasing the predicate mutex; existing predicate-based waiting prevents
lost wakeups. Events arriving during dispatch enter the next batch.

The WASM input call also previously built two temporary vectors for every
dispatched event: integer arguments and converted Wasmtime values. Both are now
fixed stack arrays, sliced to the original ABI argument count. Absolute and
relative motion, buttons, wheel, keyboard, UTF-8 text, focus, size and fullscreen
retain their original arguments. UI callback pacing is unchanged.

## Network

Each detached SCTP channel keeps its reusable 1 MiB read buffer. Native audio and
video continue to bypass the control receipt queue. Control messages (and media
receipts in isolated transport probes) require owned bytes because the next read
overwrites the buffer. That necessary copy now precedes acquisition of the shared
connection mutex. Validation, accounting and queue publication still happen under
one lock acquisition; publication transfers ownership without another copy.

The immutable session mode determines which channels need a receipt copy. The
1 MiB message, 16-message and 4 MiB queue bounds, failure behavior, ordering and
cancellation checks remain in place. A rejected/full/retired receipt can incur
one bounded copy before its state is checked; it is never queued after retirement.
No additional task, timer, media buffer or decoded-frame copy is introduced.

## Evidence and limits

Synthetic tests exercise repeated full FIFO batches, stable reuse of exactly two
queue allocations, concurrent producer progress while a batch is consumed,
actual calls into a small WASM ABI fixture, receipt ownership, cancellation and
queue budget enforcement. Existing input-wakeup, media-bypass and WebRTC/TURN
transport tests remain relevant.

These establish removal of specific allocations and byte-copy work from critical
sections, not a measured reduction in end-to-end stream latency. Hardware decode
timings and real-network jitter need separate measurements. In particular, this
change does not add UDP batching, change decoder reordering, rewrite SPS/VUI,
enlarge socket buffers or modify SCTP reliability or ICE/TURN routing.
