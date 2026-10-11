# Native media buffers and video recovery

Audio reuses compressed Opus packets and decoded PCM vectors, with at most 32
spare buffers of each kind. The existing packet/PCM queue limits remain intact.
PCM is written directly as atomic little-endian bytes to checked guest memory,
without an intermediate byte vector. Failed bounds checks retain the queued PCM.
Guest audio writes remain serialized with cancellation; decoder work does not
hold the connection mutex.

Video reuses **compressed** byte vectors only after the decoder worker has
copied them into its Media Foundation input sample. Spare vectors are capped at
32 and 8 MiB in total capacity, separately from the existing 32-frame/8-MiB
active queue. NAL inspection, input copying, decoder submission and video statistics snapshots
run outside the connection mutex. Cancellation invalidates captured senders and
does not join the audio worker while holding that mutex.

Media Foundation sample buffers retain their original allocation/lifetime rules.
We do not overwrite an input buffer still referenced by a transform. The
[documented input-stream flags](https://learn.microsoft.com/en-us/windows/win32/api/mftransform/ne-mftransform-_mft_input_stream_info_flags)
only permit immediate sample-buffer reuse when `DOES_NOT_ADDREF` is advertised;
this change does not assume it. Decoded GPU textures still go directly to the
existing D3D11 conversion/presentation pass. There is no new decoded-frame copy,
readback, display queue policy or codec capability.

## Recovery policy

- A compressed queue overflow, `ProcessInput`/`ProcessOutput` error or runaway
  output iteration starts recovery. Capability, device, format, renderer and
  unexpected codec-change failures remain terminal.
- The queue drops dependent pictures and waits for a new random-access picture.
  A generation boundary also rejects a packet copied concurrently with reset.
- The worker releases the old decoder session and recreates the same supported
  low-latency D3D11 path. A bounded cache contains only parameter NAL units,
  never saved pictures: up to 64 distinct units and 256 KiB, preserving updates.
  An incomplete/overflowed cache is never replayed; fresh stream parameters must
  then arrive before decoding can resume.
- There are at most three restart attempts per connection, at most one refresh
  request per second, and five seconds to resume decoded output per attempt.
  Successful output clears the waiting state, without resetting the retry budget.
- Refresh sending is asynchronous and limited to one second, so a stalled send
  does not hold up input dispatch. Failure stops recovery, not the connection's
  audio/control paths. Disconnect/cancellation stops further recovery.

The refresh packet is a **private Parsec contract**, not an RTP/RTCP WebRTC
keyframe request. It is isolated in `control::Config::keyframe_request`: opcode
13, arguments `(0, unchanged startup codec flags, 1)` on channel 0. This is
corroborated by
[lowlat's pinned WebRTC client](https://github.com/nomi-san/lowlat/blob/7b4dc4ae79990ea8c59a108c80b3c89b695c3cb4/examples/web-client/src/control.ts).
It does not change the pinned WASM, ICE routes, SCTP ordering/reliability or
advertised codec preferences. Stock-host acceptance still requires live testing;
the client times out explicitly if no usable fresh picture arrives.

Unit tests cover buffer ownership/reuse, cancelled delivery, PCM bounds/byte
order, recovery request framing, queue invalidation, request spacing and timeout
limits. A diagnostics-only synthetic D3D11 test restarts the decoder and supplies
a keyframe without SPS/PPS to check replay of saved parameters. This test uses
no account, network or real desktop content and is absent from the normal binary.
It reports `supported: false` only when the device lacks the D3D11 video
interface (`E_NOINTERFACE`), as on hosted Windows CI runners. This is not a
successful decoder recovery. On a machine with that interface, any decode,
restart or cleanup failure fails the check. The separate diagnostics executable
can run the same fixture with `video-hardware-probe report.json --recovery`.
