# Experimental native audio options

The Bluetooth delay reported on 2026-10-16 disappeared with wired output.
These options are independent experiments, not a fix or measured reduction for
that Bluetooth delay. Both default off. The proprietary WASM stays unmodified.

## Evidence and protocol boundaries

The [official buffer setting](https://support.parsec.app/hc/en-us/articles/32361377721364-Audio-Buffer-Setting)
documents minimum 50 ms and maximum 100 ms. The minimum is a playback/refill
reserve, not an extra wait imposed on every packet. Lower limits trade jitter
tolerance for less queued audio. The published configuration reference describes
`network_raw_audio=1` on the host and all clients for uncompressed stereo audio:
[official configuration reference](https://support.parsec.app/hc/en-us/articles/32381443626516-All-Advanced-Configuration-Options).
Neither page exposes the closed-source native client's exact current algorithm.

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
