# Native audio buffering and live gap investigation

## Evidence and regression

The pinned core's `main_audio_loop` drains `parsec_web_poll_audio`, then sleeps
1 ms (20 ms on the inactive/reset path). The outer callback retains only a
1 ms minimum guard. Its guest cleanup and pthread-exit epilogue remain intact.

The 60c9440 live trace had zero Opus decode errors and zero dropped decoder
packets, a maximum guest polling gap of about 14.8 ms, and a maximum WASAPI
worker gap of about 19.4 ms. Output recorded 22 starvation refills, 56 overflow
recoveries and 218,400 discarded PCM frames (4.55 seconds). The user reported
longer interruptions after that build. Requiring a fresh 75 ms refill after
every underrun and trimming bursts down to that minimum was unsuccessful and
has been removed. Startup and explicit resets still use the requested minimum.

Some intervals contained no incoming audio; other intervals delivered more than
one second's PCM in a second. The lifetime maximum packet gap was about 3.74 s.
The user could not confirm when the source was paused. These observations do
not prove a host, network or transport fault, and quiet intervals must not be
counted as confirmed audible glitches.

## SDK and lowlat comparison

- [SDK client example](https://github.com/mbroemme/parsec-sdk/blob/14222ac25be3bc28c49b4858213a737642031513/examples/client/main.c):
  a separate audio polling thread passes interleaved PCM and **frames per
  channel** to Matoya; its requested buffer limits are 75/150 ms.
- [Matoya Windows audio](https://github.com/snowcone-ltd/libmatoya/blob/952f2a22bdb4cf832db110c00aa3e57952358eae/src/windows/audio.c):
  PCM is written directly into a shared WASAPI endpoint with spare capacity,
  before starting playback at the minimum threshold. Its capacity is larger
  than the usual amount of queued audio. Native `GetQueued` reports milliseconds;
  the **pinned browser adapter reports frames**, so our WASM import must retain
  that pinned ABI. SDK signatures cannot replace the pinned web ABI blindly.
- [lowlat web session](https://github.com/nomi-san/lowlat/blob/7b4dc4ae79990ea8c59a108c80b3c89b695c3cb4/crates/net/src/web/session.rs)
  and [Opus encoder](https://github.com/nomi-san/lowlat/blob/7b4dc4ae79990ea8c59a108c80b3c89b695c3cb4/crates/audio/src/encode.rs):
  the web path sends headerless media payloads over reliable, ordered SCTP
  streams. Its Opus source uses 20 ms, 48 kHz stereo frames. It is a **host**,
  not a reference implementation of a Windows client jitter buffer. No native
  BUD audio headers, RTP headers, artificial sequence numbers or unreliable
  channel policy are introduced into our existing web-compatible connection.

These are behavior references; no SDK binary or copied application source is
added. Our pinned WASM and Opus negotiation remain unchanged.

## Output policy

The previous IAudioClient3 endpoint held only 1,056 frames (22 ms) on the user's
device, leaving most of the 75 ms reserve in a Rust queue. A local worker stall
could therefore exhaust the endpoint while PCM remained outside Windows.
The output now uses documented `IAudioClient::Initialize` shared event mode
with capacity requested from the existing maximum budget. Capacity is **not**
prefilled with silence or used as a target latency. Actual PCM is staged before
`Start`, and startup begins after the same minimum threshold as before.

The producer also signals the owned kernel event, so data can wake a stopped
endpoint without a polling delay. Kernel handles have shared lifetime ownership;
all COM interfaces and shutdown remain on the rendering worker. MMCSS and device
format conversion are retained. The engine period is queried rather than forced.

The renderer budgets **software frames plus endpoint padding together** against
the maximum. Staging memory is separately bounded. On overflow it discards only
the excess oldest staged stereo frames, without dropping the whole reserve or
restarting playback. Once started, a real underrun resumes with the next PCM
instead of deliberately inserting another minimum-length wait. This trades the
failed repeated-refill policy for prompt recovery; it cannot manufacture audio
that has not arrived. No extra resampler, silence queue or DSP is added.

Increasing endpoint capacity alone does not establish lower latency or resolve
upstream stalls. The live result remains to be tested on the affected device.

## Validation and diagnostics

Tests verify partial PCM writes, startup versus immediate underrun recovery,
combined padding/staging accounting, excess-only stereo trimming and reset.
A deterministic simulation runs an independent 10 ms engine and 20 ms source
through repeated 40 ms rendering-worker stalls, checking every stereo sample for
continuity and enforcing the existing total budget. Windows CI also exercises
the silent native endpoint probe and guest cleanup.

Opt-in trace schema 3 adds per-sampling-interval packet-gap maxima, time since
the last completed audio message, receive-to-decoder handoff, decoder queue wait
and decode duration. This separates local handoff/decode stalls from gaps before
the completed SCTP-message boundary; it does not locate the origin of those
earlier gaps. Only times and counts are recorded, never media or credentials.

Supported API references:

- [Microsoft: IAudioClient::Initialize](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize)
  (shared capacity versus actual playback latency).
- [Microsoft: Rendering a Stream](https://learn.microsoft.com/en-us/windows/win32/coreaudio/rendering-a-stream)
  (stage data before Start; use current padding and partial writes).
- [Microsoft: GetCurrentPadding](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getcurrentpadding).


## Direct-connection test: receive bursts (2026-10-12)

The `8526906` live trace still stuttered. The user reports continuous sound with
stock Parsec to the same host, and a direct ICE connection in our client. The
completed-audio-message gaps repeatedly reach 150–535 ms (maximum 1,080 ms),
while local handoff peaks at 0.049 ms, decoder queue wait at 4 ms, and renderer
wake gap at 13.85 ms. One interval receives no audio, followed by a catch-up
burst. This places the largest observed gap before the completed SCTP read;
it does not distinguish host web-path behavior, packet loss, retransmission or
transport scheduling. Stock native transport is not an identical control path.

The next controlled change requests a **1 MiB minimum UDP SO_RCVBUF** on native
host, server-reflexive and UDP TURN sockets. Larger existing buffers stay intact;
OS limits and option failures remain nonfatal. TCP/TLS TURN streams, ICE port
selection, reliable/ordered channels, decoder and audio playout stay unchanged.
This is burst capacity, not a wait-to-fill policy. It may reduce kernel drops
when video arrives in bursts, but packet loss has not yet been established and
this is not a confirmed cure. No additional audio buffering is introduced.

The optional IO trace records requested, previous and actual socket buffer sizes
and option failure, without addresses or credentials. A loopback test checks
readback, no shrinking and intact datagrams. Existing ICE/DTLS/SCTP and UDP/TCP/TLS
TURN integration probes continue to cover the gather paths.

Supported APIs:
- [socket2 SockRef](https://docs.rs/socket2/0.6.5/socket2/struct.SockRef.html)
- [Winsock setsockopt / SO_RCVBUF](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-setsockopt)

The lowlat socket implementation at the revision above explicitly provisions
receive capacity for bursts. Its Linux host's 64 MiB request is not copied into
this Windows client; neither its recvmmsg loop nor its native transport is used.
