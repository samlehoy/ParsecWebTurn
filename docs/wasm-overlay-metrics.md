# Unmodified WASM overlay metrics

The pinned `parsec_web_get_metrics` host import has exactly seven output pointers:
width/height (u32), 4:4:4/full range (u8), and decode/encode/network (f32 in ms).
The reference `weblib.js` writes these from `clientGetMetrics`. Its WebCodecs
decoder measures submission to an output callback/animation-frame callback;
it does not time GPU execution. Its network value is left zero. The former
native import also left decode and network zero, which the core renders as N/A.

The native bridge now supplies:

- Decode: the latest existing native input-to-observed-decoder-output duration,
  only after a correlated sample and a presented picture. No fallback to decoder
  API CPU service time or GPU-time inference. No new frame timing or GPU query.
- Encode: the existing host-reported encode duration, unchanged.
- Network: a fresh active-transport RTT, in milliseconds. BUD reuses its existing
  transport snapshot. WebRTC reuses the existing one-second telemetry task and
  selected-pair statistics, even with native Connection stats closed. Detailed
  native stats publication still depends on visibility/logging. No per-frame
  network query, new worker, network probe or reliability/timer change is added.
  The cached value expires after three seconds; unavailable, invalid or inactive
  measurements retain the zero/unavailable sentinel. This is additional bounded
  WebRTC telemetry work, not a claim of zero CPU overhead.

Parsec describes the overlay's Network latency as round-trip latency in
[its overlay documentation](https://support.parsec.app/hc/en-us/articles/32381603663636-Stream-Overlay-Stats-and-Logging).
The stock core's formatter shows N/A at values <= 0.01; very small measured
durations are not artificially increased to force a numeric label.

## Bitrate boundary

The inspected web overlay clears a 736-byte status structure before the import.
Decode/encode/network fill offsets 32/28/36 in that structure; the bitrate row
reads offset 40, which has no argument in the web import and remains zero.
The browser reference likewise has no bitrate output in `weblib.js`.

The bridge deliberately does not write beyond those seven caller-provided
outputs, derive adjacent private pointers, patch WASM instructions or mutate
private core status fields. The embedded overlay's Bitrate therefore stays N/A.
Native Connection stats continues to provide its separately labelled incoming
and outgoing traffic rates; those include more than encoded video bitrate.

Offline tests exercise the real import with a synthetic WASM caller, float
units, packed byte fields/padding, inactivity, and stale/invalid RTT. Live overlay
display still needs user validation with the resulting client build.
