# Input and audio troubleshooting capture

The `--trace-io` switch records a targeted diagnostic session to
`io-diagnostics.jsonl` beside the executable (or inside the explicit `--data-dir`).
It is off by default, requires no administrator privileges and makes no capture,
audio buffering or video pipeline policy changes. File writing runs on a separate
thread, once per second, for at most ten minutes. Starting another capture replaces
the previous file. Close the client normally to finish the capture.

The header identifies the app version, pinned core and CI build commit. Records
contain only input mode and focus/capture states, cursor flag counters, documented
Windows API error codes, Opus decoder counters and WASAPI queue/scheduling counters.
They do not contain media payloads, cursor positions/images, key values, typed
text, account identifiers, credentials, network addresses or device names.

## Reproduction

1. Start the included `Start-IO-Diagnostics.cmd` test launcher or run
   `ParsecWebTurn.exe --trace-io`.
2. Connect to the affected host. Play continuous audio for about a minute;
   note roughly when popping or interruptions occur.
3. In the stream overlay, select Immersive Mode → Mouse, then Both. Close
   the overlay and try moving the pointer beyond the client edges. Test Off
   separately: Off must allow leaving the window unless the host requests
   relative input. Briefly switch focus away and back, then repeat Mouse.
4. Disconnect normally, close the client and return `io-diagnostics.jsonl`.

## Current trace schema 5

Playback now uses the original shared WASAPI policy; schema-3 refill/overflow
semantics below are historical. The playback snapshot again contains only
original output counters: real frames queued/written/dropped, underruns, staging
frames, endpoint padding, device-open/failure/cleanup state. `underruns` counts
partial writes that enter refill buffering, not audible clicks.

The header adds `started_unix_ms` and `audio_output_policy`; every record adds
`unix_ms` derived from the initial wall-clock anchor and monotonic elapsed time.
For cross-machine captures verify clock agreement separately; do not infer
one-way latency from unverified clocks. Schema-4 files have no absolute anchor
and cannot be aligned precisely using file modification time.

See [Host packet timing comparison](host-audio-transport-comparison.md).

## Interpretation and limits

`input.state` is the policy applied by the Rust window. `events.core_reads`
compares the applied mode with a fresh read of shared pinned core configuration
and records missing/invalid app contexts. Schema 1 called this field `cached_mode`;
schema 2 uses `applied_mode` after eliminating the instance-local cache. A missing
app context in the GUI worker's keyboard import is expected; main-loop reads
must have a valid app context.

`clip_owned` is the client's last successful ClipCursor ownership flag;
`clip_inside_client` independently queries the current Windows clipping rectangle.
Focus comes from GetGUIThreadInfo for the window's UI thread, not the sampler's
thread-local GetFocus. Host relative/hidden cursor signals are recorded separately
from immersive confinement. Raw absolute device events are counted separately
because the current relative adapter accepts delta packets only.

Audio decoder backlog (`queued_samples`, divided by 96 for milliseconds) and
playback backlog (`queued_frames` and `device_padding`, each divided by 48)
identify different queues. `max_poll_gap_ms` measures guest audio polling gaps;
`max_worker_gap_ms` measures WASAPI worker wake gaps. `max_packet_gap_ms` measures
gaps between completed incoming Opus messages (including source pauses).
Schema 3 adds `interval_max_packet_gap_ms`, reset after each diagnostic sample,
and `packet_silence_ms`, the current age of the last incoming message. A quiet
interval is therefore distinguishable from an old lifetime peak.
`max_receive_handoff_ms` covers completed read to decoder submission, including
the connection lock; `max_decode_queue_wait_ms` covers submission to worker
pickup, and `max_decode_time_ms` covers decoding and PCM-buffer acquisition.
These maxima are cumulative. They do not include socket/DTLS/SCTP reassembly time.
`empty_device_wakes` also counts an empty endpoint when PCM is already staged.
In schema 1 `underruns` counted repeated empty wakes; schema 2 counted forced
refills. Schema 3 counts transitions into an observed empty endpoint and empty
staging queue, without forcing another refill pause. `buffering` now denotes
startup/explicit reset only. The obsolete `rebuffer_events` and
`low_latency_shared_mode` fields are removed; `native_shared_buffer` identifies
the endpoint buffering path. `overflow_recoveries` now counts excess-only trims
against the combined endpoint/staging maximum. Counters across schemas are not
directly comparable.
Empty-device observations indicate starvation,
not a precise count of audible clicks. Startup buffering, mute/reset, focus
changes and a silent/stopped source must be considered when interpreting them.

This capture cannot establish end-to-end audio latency or audible quality by
itself. Reproducing the symptoms on the affected machine remains necessary.
