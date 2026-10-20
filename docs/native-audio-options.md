# Experimental native audio options

The Bluetooth delay reported on 2026-10-16 disappeared with wired output.
These options are independent experiments, not a fix or measured reduction for
that Bluetooth delay. All custom Audio options default off. The proprietary WASM stays unmodified.

## Evidence and protocol boundaries

The [official buffer setting](https://support.parsec.app/hc/en-us/articles/32361377721364-Audio-Buffer-Setting)
documents minimum 50 ms and maximum 100 ms. The minimum is a playback/refill
reserve, not an extra wait imposed on every packet. Lower limits trade jitter
tolerance for less queued audio. The published configuration reference describes
`network_raw_audio=1` on the host and all clients for uncompressed stereo audio:
[official configuration reference](https://support.parsec.app/hc/en-us/articles/32381443626516-All-Advanced-Configuration-Options).
Neither page exposes the closed-source native client's exact current algorithm.

The pinned web core requests 75/150 ms in the tested automatic session, despite
the native client's documented 50/100 ms defaults. The Audio menu starts custom
values at 75/150 for consistency with that core; existing saved overrides are
preserved. With custom buffering off, those fields are inactive and stats shows
the limits actually requested by the core.

Reviewed lowlat `feat/win-seams` revision:
`8674882319b24023547bbed77821c472566af69b`.

- `examples/client/sound.cc` uses a 75/150 ms playback floor/ceiling in SDL;
  it pauses/clears and refills after empty/overfull output. This is example policy,
  not an SDK-imposed latency or proof of stock-client defaults.
- `crates/client/src/sound.rs` acquires and decodes on the caller's thread, from
  a bounded packet pool. The pool's 32-slot capacity is not a deliberate wait.
- `crates/core/src/init.rs` declares `rawAudio` as a permission; the host can
  still send Opus. The 15-byte BUD audio header identifies PCM as codec 2 and
  carries layout/rate; other codec tags are interpreted as Opus by that contract.
- The reviewed web session drops the audio header before SCTP delivery. Our
  current WebRTC contract consequently cannot safely accept raw plus an Opus
  fallback. No payload-size/content heuristic is introduced. Raw is BUD-only;
  enabling its dormant setting never changes WebRTC startup or channel policy.

## Full local path and decisions

### Multichannel BUD extension (2026-10-20)

**Validation status: synthetic tests only; not validated in a live multichannel
host-to-client session.** The isolated Opus/PCM tests and silent 5.1/7.1 WASAPI
output probe have passed. These results do not establish live channel ordering,
audible surround playback or physical speaker mapping. Live validation is
deferred; this feature remains experimental and disabled by default.

Parsec documents automatic 5.1/7.1 support in its normal client, without a paid
plan requirement on the feature page:
[Multi-Channel Audio](https://support.parsec.app/hc/en-us/articles/46395533300372-Multi-Channel-Audio).
The host and client speaker layouts must agree; otherwise the host sends stereo.

The optional **Multichannel audio (experimental, BUD)** setting reads the default
Windows render endpoint's WAVEFORMATEXTENSIBLE channel count and speaker mask.
Only 5.1 back (`0x3f`), 5.1 side (`0x60f`), and 7.1 (`0x63f`) are accepted. A
temporary shared-mode client validates 48 kHz/16-bit PCM initialization without
starting or writing audio. Query/initialization failures request stereo. Both
initial and relay retry attempts declare the same selected layout. No host
configuration is changed automatically.

Every BUD packet remains authoritative: rate, channel count/mask and codec are
validated before decoding. The selected layout and stereo fallback are allowed;
other layouts are rejected. The existing bundled libopus supplies multistream
decoding; no dependency is added. Windows-order mappings are protocol facts
checked against the installed host's multistream configuration: 5.1 uses four
streams/two coupled pairs with `[0,1,4,5,2,3]`; 7.1 uses five/three with
`[0,1,6,7,2,3,4,5]`. Decoder history and staged PCM are retired on layout changes.
All budgets and stats count frames, independent of channel count.

The pinned WASM's audio loop is fixed to stereo. Its PCM poll still receives a
valid stereo reference mix. For surround only, a bounded native handoff retains
the original interleaved channels alongside that polled block. The corresponding
MTY_AudioQueue import must match the exact guest pointer and frame count before
it can consume that block once. The next poll retires an unqueued block, so core
mute/discard decisions cannot replay old sound on unmute. Native reset, destroy,
disconnect and cancellation retain the normal lifetimes. No source patches,
guest structure-offset writes, synthetic silence tokens, alternate session SDK
or GPU changes are introduced.

WASAPI receives the original channels through a WAVEFORMATEXTENSIBLE PCM stream.
A format change replaces only that audio context under its existing handle;
failed surround initialization keeps the stereo reference playback and records
a bounded failure for that layout. The same failing layout is not retried for
every packet. Core mute and buffer controls remain in charge. Stereo PCM keeps
the existing path; surround adds the reference fold-down/guest transfer needed
by this unchanged ABI, with no extra deliberate buffering. This is not a claim
of lower CPU cost or latency than stereo.

Connection stats reports received channels and, if different, playback channels.
The event log contains layouts and failures, never audio content. WebRTC retains
its headerless stereo contract, including startup, TURN and SCTP behavior.
Actual device channel ordering, audible mute/unmute and host stereo fallback
still require live testing with a matching surround endpoint and host source.
Synthetic tests cover separate Opus channel signals, PCM handoff ownership,
discard-on-mute behavior, stereo transitions and bounded playback queues.

#### Testing without physical surround speakers

The isolated tests generate independent signals for every channel and verify
Opus decoding and PCM ownership without any audio device. The existing
`native-audio-probe` diagnostic additionally queues silent 5.1/7.1 PCM into
WASAPI and reports initialization and playback errors. Windows may downmix that
input on a stereo endpoint: successful API playback is not proof of distinct
physical speakers or of a multichannel host session.

For a complete stock-host test, configure matching virtual speaker endpoints
on both machines as 5.1 or 7.1, then reconnect using BUD and the experimental
multichannel option. For example, the documented speaker endpoint of
[VB-CABLE](https://vb-audio.com/Cable/VBCABLE_ReferenceManual.pdf) supports up to
eight channels. The host source must output a known signal separately on every
channel. Record the client's virtual cable output at the same channel count
and inspect each recorded channel independently, including LFE; check stats
for the actual received layout and verify mute/unmute and stereo fallback.
Listening to a stereo mix alone cannot establish channel identity. Driver
installation, endpoint selection and Windows speaker settings are manual;
the application never changes them. Physical surround perception remains a
separate test requiring suitable speakers or a verified spatial-audio renderer.

1. WebRTC SCTP or BUD reassembles audio. Existing transport reliability,
   timers, routing and retransmission policy are unchanged. Buffer controls
   cannot remove transit or ordered-stream recovery delays.
2. A bounded native packet queue wakes the decoder immediately on arrival;
   `recv_timeout(20 ms)` is a maximum idle wait, not a per-packet sleep. The
   decoder worker now requests nonfatal thread-scoped MMCSS audio scheduling.
3. Opus still uses the reference decoder. BUD PCM is validated as nonempty,
   at most 32,000 bytes and complete interleaved stereo frames, then converted
   from little-endian i16 into the existing reusable PCM storage. It bypasses
   Opus decoding. The header controls every packet, permitting an Opus fallback.
   Returning from PCM resets Opus's predictor state using the documented reset
   control; ordinary continuous Opus does not reset. Long PCM packets are split
   at the existing maximum guest audio-block capacity. The same capability
   factory supplies the initial BUD attempt and an automatic relay retry.
4. Decoded PCM remains bounded. Automatic mode retains its 500 ms storage
   ceiling (capacity, not target delay); custom mode bounds this staging queue
   to its chosen maximum. On backpressure the oldest decoded blocks are dropped.
   Opus is still decoded in order to preserve predictor state.
5. The pinned WASM audio polling/queue ABI and existing mute/overlay controls
   remain in place. Removing the PCM roundtrip would require a separate control
   integration and is not part of this change. Video/GPU work is unaffected.
6. Automatic WASAPI playback retains the existing policy. Experimental custom
   playback tries `IAudioClient3::GetSharedModeEnginePeriod` and
   `InitializeSharedAudioStream` at the reported minimum. If unavailable or
   rejected, a fresh audio client uses the existing 20 ms initialization request.
   Actual endpoint capacity is measured; the requested duration is not a latency
   measurement. WASAPI may perform its normal device-format conversion.
   During startup/refill the custom worker checks the PCM reserve with a bounded
   5 ms wait; once playing it is paced by WASAPI events. This is a remaining local
   scheduling granularity, not a claimed zero-delay handoff.
7. Custom playback starts with real queued PCM and writes only available real
   frames. There is no prefilled or repeated silence reserve. It refills after
   a real underrun. The output budget counts its staged PCM plus sampled WASAPI
   padding and keeps newest complete stereo frames on overflow. Already submitted
   device audio cannot be removed by trimming the software queue. Decoded staging
   is separately bounded and displayed; the maximum is not a single global budget
   for all transport/decoder/device queues.

The selected minimum cannot create audio that has not arrived. Smaller settings
are not guaranteed faster, and raw removes codec work, not packetization or
network waiting. Raw stereo payload is 48,000 * 2 * 16 = 1.536 Mbps before headers.
No host settings are changed automatically.

## Verification

Regression coverage checks safe automatic defaults and invalid limits, UI tab
switch/save behavior, header-based PCM/Opus distinction, malformed stereo PCM,
exact signed PCM extremes and an Opus fallback, output overflow with device
padding and preservation of the original automatic queue policy. Diagnostics
exercise automatic and experimental output with silent local PCM, isolated
settings and no account/network. Live host raw negotiation, audible stability
and end-to-end delay still require user testing.

The stats/log fields report the actual decoded codec and local queues. They
exclude network transit, host capture and Bluetooth/headset buffering. There
are no raw packets, sample contents or credentials in the operational log.

The [local silent WASAPI report](measurements/audio-wasapi-2026-10-16.json)
records automatic and custom 20/60 ms probes. Both wrote all 9,600 submitted
frames with zero drops, no API failure and released resources. The low-period
API request was accepted in the custom case, but device capacity was 1,056
frames (22 ms) in both cases. One underrun was counted in the custom probe;
this short finite source includes ending and scheduling effects. Its automatic
0/200 ms limits are diagnostic inputs, not a measurement of the core's defaults.
This proves initialization/output/cleanup, not lower end-to-end latency or
better audible stability. The WASAPI implementation is identical between the
recorded source and the final audio test client; later changes concern PCM
packet splitting before the guest boundary.

Supported API reference:
[Microsoft shared low-latency initialization](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient3-initializesharedaudiostream).
