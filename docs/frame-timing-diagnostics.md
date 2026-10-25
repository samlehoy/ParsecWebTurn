# Stock-host frame timing diagnostics

Investigated on 2026-10-20; native diagnostic collector added on 2026-10-21.
The host field mapping was investigated statically. User screenshots on
2026-10-21 confirm live BUD report reception; live WebRTC validation remains
pending. Neither proprietary binary was modified.

Inputs:

- Pinned WASM 150-104a, SHA-256
  `d663dd96df477c65479fb93eb88756c7fcafff581cc93be563625cd195a4b4a6`.
- Installed stock Windows host `parsecd-150-105c.dll`, SHA-256
  `dbd59739cfb84a239860a30480b91281f0ebd316dfb5bf58bd4539e82c92c425`.
- [lowlat protocol, revision 5f8db568](https://github.com/nomi-san/lowlat/blob/5f8db5689e55de0d396b0520824caa9fc053dd86/docs/01-protocol.md),
  sections 11.1 and 11.2. The current `feat/win-seams` description was also
  checked: it still identifies bit 1 as an unknown second diagnostic.

## Request and response contract

The normal control header contains three big-endian 32-bit arguments followed
by one opcode byte. Opcode 35 (`0x23`) takes a bitmask in argument 0:

| Mask | Meaning in the inspected stock host |
| --- | --- |
| `0x00` | Disable both timing diagnostics |
| `0x01` | Enable host video timing |
| `0x02` | Enable host audio timing |
| `0x03` | Enable both |

Arguments 1 and 2 are zero and there is no request body. This is a set operation,
not a toggle. A later zero request clears both flags.

The previously unidentified bit 1 has a concrete audio path in this build.
Both paths reply with opcode 34 (`0x22`):

| Argument 0 | Argument 1 | Argument 2 | Body | Control message size |
| --- | --- | --- | --- | --- |
| `0` (video) | stream index | `0` | Four big-endian IEEE-754 `f32` values | 29 bytes |
| `1` (audio) | `0` | `0` | Three big-endian IEEE-754 `f32` values | 25 bytes |

Values are **durations in milliseconds**, not absolute timestamps. The stock
timer reads QueryPerformanceCounter and divides elapsed ticks by
QueryPerformanceFrequency / 1000 before narrowing to floats for serialization.

Video fields, in wire order, correspond to host loop total, capture/pre-encode
segment, the host's encode-duration measurement, and frame interval. The capture
segment includes the host's work/waits between its capture call and the encoder
output stage; it must not be described as a pure GPU capture execution time.
The interval uses successive processing-completion timestamps, not display
presentation timestamps.

The initial native UI incorrectly reversed the last two labels. Rechecking the
producer and serializer on 2026-10-21 establishes that the third field comes from
the same duration used by the host's smoothed opcode-21 encode report, while the
fourth comes from the previous completion timestamp (updated each iteration).
The serializer preserves this order. The timer divides ticks by frequency/1000;
no factor-of-ten correction is needed. UI regression assertions now check both
fields' latest, average and peak values independently.

The live screenshot's stale audio report and very large audio interval peak are
not explained by the video label correction. Audio startup/idle interval behavior
still needs investigation; these values must not be called playback latency.

The three audio producer values are a common audio-processing duration, a
short timing span, and a successive-block interval. The matching WASM graph
labels suggest total/encode/frame interval, but this naming is an interpretation:
the stock client's complete wire-to-graph mapping was not traced. **The middle
slot must not be advertised as the complete Opus encoding cost:** the inspected
common audio loop populates it before the later per-peer codec operations in
the audio send path. Its usefulness and observed values still need live validation.

## Static evidence and reproduction anchors

These RVAs identify locations in the exact stock-host DLL above; they are
research references, never runtime addresses or integration dependencies.

- Request builder: RVA `0xA5E10` combines two boolean inputs into bits 0 and 1
  and serializes a 13-byte opcode-35 control message.
- Host control dispatch: the opcode-35 table entry leads to RVA `0xAE7BC`,
  storing the two flag bits at host-state offsets `0x2F1C` and `0x2F1D`.
- Video producer: diagnostic gate at RVA `0xACCB1`; timing fields constructed
  through `0xACCFE`. Video serialization starts at `0xAA49D`, sets argument 0
  to zero, and sends a 13-byte header plus a 16-byte body.
- Audio producer: diagnostic gate at RVA `0xAD502`; three timing values are
  constructed before the audio send call. Audio serialization is gated at
  `0xAACC8`, sets argument 0 to one, and sends a 13-byte header plus 12 bytes.
- Timer conversion: RVA `0xD9410`; the frequency divisor at `0x373CE8` is 1000.
- The audio send path performs per-peer audio codec work before sending the
  already constructed timing values, which is why its middle field needs the
  interpretation caveat above.

The flags are stored in the common host state used by the video/audio loops,
and the send routines walk active peers. They do not appear to be independent
per-requester subscriptions. Another client's diagnostics request could change
the shared state; exact multi-client interaction needs a live test.

## What the unchanged WASM establishes

The function named `overlay_update_metrics_advanced_graph_metrics_data*` has
labels for Host/Client Video Total, Host Video Capture, Host Video Encode,
Host/Client Video Frame Time, Host/Client Audio Total, Host Audio Encode,
Client Audio Decode, and Host/Client Audio Frame Time. This corroborates the
purpose of the two telemetry families; labels alone do not establish a working
web transport path.

The web module delegates transport and aggregate metrics through imports such
as `parsec_web_send_message` and `parsec_web_get_metrics`. The latter has seven
output pointers for resolution, color flags and aggregate decode/encode/network
metrics, without the detailed per-frame payload above. The reviewed accompanying
public browser JavaScript handles opcode 21 but has no opcode-34 handler.
The existing WASM graphs therefore do not automatically expose these reports.
A native collector can implement the protocol without patching the WASM or
writing its private graph structures.

## Native integration (2026-10-21)

Our BUD driver's ordinary startup still sends opcode 35 with all bits clear.
The native BUD consumer and WebRTC control reader now collect opcode 34 before
their application-control queues. A single native collector validates the wire
formats and retains only count, latest, online mean, peak and receive time for
each of five video streams and audio. There is no per-frame history or file
logging. Unknown variants and malformed/nonfinite/negative values are counted
and discarded without terminating the connection. The existing Host encode
time remains the separate opcode-21 report.

At the user's request there is no additional settings switch. Collection follows
the existing diagnostic demand: Connection stats is visible or event logging is
enabled. Once control startup is complete, a transition sends mask `0x03` through
the existing transport; when demand ends it sends `0x00`. Close attempts a bounded,
best-effort disable before normal teardown. A failed diagnostic request is reported
as unavailable and stops diagnostic retries for that attempt, without failing
the media session. Transport loss cannot guarantee that the disable reaches the
host. No ICE/SCTP/BUD timers or reliability policies are changed.

Connection stats displays latest/average/peak values in milliseconds, sample
counts and the age of the last report. A report older than three seconds is
marked stale; absent values are "Not reported by host", never a synthetic zero
or proof that the host does not support the feature. Video total, capture
segment, frame interval and encode duration are displayed. Audio total and
block interval are displayed; the uncertain intermediate field remains hidden.
Statistics are reset on a new collection/connection, and generation checks
prevent old workers from clearing or publishing into a newer connection.

Validation and interpretation constraints:

- Consume reports before the bounded application-control queue and retain only
  bounded samples/aggregates. At 60 fps, video telemetry alone adds 1,740 bytes/s
  of control header and payload, excluding transport/crypto overhead. Ordinary
  queues and synchronous per-frame file logging are inappropriate collectors.
- Keep local receive, decode and presentation timings separate from host
  durations. There is no frame ID or absolute timestamp in these messages:
  exact frame matching or full capture-to-display latency cannot be inferred
  from arrival order, particularly after loss, buffering or dropped frames.
- Automated tests cover byte order, both variants, aggregates, malformed/future
  reports, stale/missing UI values, generation isolation and 10,000-report bursts
  through both native transport adapters. These are synthetic tests, not proof
  of live stock-host operation. The user's BUD screenshots confirm reception;
  live WebRTC and audio interval behavior remain unverified.

The telemetry could distinguish host timing spikes from smooth host operation
followed by client-side stalls. It does not accelerate media or require a GPU
copy, and its CPU/network cost has not been measured. Decoder/rendering code and
the WASM remain unchanged; no new dependency or persistent setting is added.
