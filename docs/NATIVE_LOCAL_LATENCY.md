# Local latency measurements

Open **Connection stats** during a session and return focus to the stream. Move
the pointer, type and scroll. The window updates once a second. Compare the
average, p95, maximum and sample counts while idle and under normal load.

The two measurement boundaries are:

1. **Input → send (sync):** `Window::push` receives a native input event, before
   locking its queue, through the specific WASM input callback, native control
   preparation, command queue wait and successful `RTCDataChannel::send` return.
   The endpoint means acceptance by the local SCTP sender, not departure from
   the NIC. Windows message-queue time before `Window::push`, subsequent SCTP
   buffering, DTLS/socket work and the network journey are outside this metric.
   Only control sends invoked synchronously by that exact input callback carry
   its timestamp. General event-loop callbacks, worker stores, synthetic input,
   unrelated control messages and asynchronous callbacks do not inherit a last
   input timestamp. If the pinned core defers sending, no correlation is claimed.
2. **Receive → media queue:** immediately after `read_data_channel` returns a
   complete message, before acquiring the session lock, through validation,
   video framing inspection and successful native pipeline `submit`. Video and
   audio are reported separately. This includes local lock wait and compressed
   message copying/enqueueing, but excludes NIC reception, DTLS/SCTP processing,
   message reassembly, queue residence after submission, decode and playback.
   Unavailable, invalid or rejected media does not produce a latency sample.

Timings use Rust's monotonic `Instant`. Only numerical durations are retained in
memory, never input text, keys, coordinates or packet contents. Each session has
independent bounded storage: at most 1,024 samples per path from the last five
seconds. A busy path can therefore represent less than five seconds; the actual
sample count is shown. P95 is the nearest-rank 95th percentile. No samples means
unknown, not zero. Ended sessions discard these measurements. Recording occurs
only while the stats window is visible; snapshots are sorted outside the session
transport/media lock. No file logging or new dependency is added.

This change measures the current pipeline. It does not change ICE selection,
SCTP reliability/ordering, socket options, scheduling, decoder behavior or the
presentation queue. Measurements carry small instrumentation overhead and do
not establish end-to-end input-to-picture latency.
