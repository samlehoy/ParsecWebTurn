# Experimental BUD TURN TCP/TLS

Transport-only port of [lowlat PR #2](https://github.com/nomi-san/lowlat/pull/2),
revision `befd5a29c7a331111f11378c0b5623de788e9769` (draft when reviewed).
Only native carrier IO, stream framing, reliable TURN transactions and the
existing external-media client seam are reused. No SDK/C ABI, additional
decoder, host, exporter or browser transport is included. Original WASM and
GPU decoding/presentation are unchanged.

The Network tab's existing custom URLs / Cloudflare credentials supply both
transports. BUD orders available URLs as UDP, TLS, TCP port 80, other TCP,
retaining order within each tier. Ports are never rewritten. Direct discovery
still precedes TURN unless Force TURN is set. Each failed startup relay is
retired before a fresh account attempt uses the next URL. All retired attempt
IDs remain suppressed. Up to 32 relay URLs are considered. No switch to WebRTC
or migration of an established stream is attempted.

Socket/TLS setup is cancellable and bounded to 10 seconds; the existing
30-second establishment allowance applies to each attempt. An answered direct
attempt retains its four-second relay fallback. Immediate carrier failures can
advance sooner. TLS uses the original URL hostname, public/OS roots and normal
certificate validation. Private test roots exist only in isolated fixtures.

The nonblocking mio/rustls loop uses the existing session worker, bounded
framing storage, partial-write preservation, TCP_NODELAY and explicit wakeups.
There is no new runtime/thread or batching sleep in the media send path. TLS
is pinned to the PR's patched rustls 0.23.46 minimum. TCP head-of-line blocking
still applies between client and TURN; the relay-to-host allocation remains UDP.

Connection stats reports a relay carrier only after a relayed path establishes.
Traffic uses UDP payload bytes or TURN plaintext frame bytes (including padding)
accepted by the socket/TLS writer or received as complete frames. TLS/IP/TCP
headers and account HTTPS/WSS are excluded; acceptance does not prove delivery.

Validation includes framing/storage reuse, malformed/fragmented/coalesced input,
backpressure/partial writes, TLS identity/trust rejection, cancellation, UDP
regressions, and isolated authenticated coturn UDP/TCP/TLS across allocation and
nonce renewal with the client's UDP source blocked in stream cases. Synthetic
media-channel bodies verify delivery, not real decode or latency improvements.
Live stock-host/provider playback remains an acceptance test.

On 2026-10-16 the [Windows/Linux transport validation](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/38945891934)
passed, including 283 retained protocol tests, application-level direct/UDP/TCP
connectivity and all three 65-second authenticated coturn renewal cases. The
stream cases blocked the client's UDP source port. No measured real-video
latency or public-provider compatibility is inferred from these synthetic tests.
