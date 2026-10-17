# Experimental BUD TURN TCP/TLS

Transport-only port of [lowlat PR #2](https://github.com/nomi-san/lowlat/pull/2),
revision `befd5a29c7a331111f11378c0b5623de788e9769` (draft when reviewed).
Only native carrier IO, stream framing, reliable TURN transactions and the
existing external-media client seam are reused. No SDK/C ABI, additional
decoder, host, exporter or browser transport is included. Original WASM and
GPU decoding/presentation are unchanged.

The focused TLS backpressure/deadline fix from
[lowlat commit d5d8d9e](https://github.com/nomi-san/lowlat/commit/d5d8d9ed4b1208f00de9fe8f90286919b15a0240)
is applied on top (2026-10-17), together with its four regression tests.

Pending carrier work now includes both the TURN frame and rustls ciphertext;
the shell does not consume another protocol datagram until both drain. Plaintext
acceptance and partial socket writes cannot restart the oldest frame's age.
Poll waits are capped by the existing five-second write deadlines, including
TLS-only writable work, while control records produced after idle start a fresh
deadline. The TLS buffer remains capped at 32 KiB. These bounds cover local
application/TLS queues, not kernel/relay queues, TCP head-of-line blocking or BUD
retransmissions. No measured latency improvement is claimed.

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

Live acceptance on 2026-10-16 confirmed TURN/TLS to Metered with a stock host,
using AMD AMF HEVC and Direct D3D11 HEVC. Both connections established in about
two seconds without logged transport or decoder errors. This establishes
compatibility for that setup, not a measured latency improvement or Main 10 proof.

## Low-latency and release audit (2026-10-16)

- Input producers explicitly wake the existing session worker. Socket readiness
  interrupts its wait; the 50 ms safety cap and 100 ms consumer lifecycle check
  are not media batching intervals. Buffered stream/TLS input and fairness yields
  schedule a zero-timeout continuation, interleaving receive, input and deadlines.
- TCP_NODELAY is enabled after the documented nonblocking connect sequence and
  before TLS/application writes. Small control/input frames do not wait for a
  Nagle batch. Both TCP and TLS roundtrip regressions check the socket option.
  See [Mio's connection and NODELAY contract](https://docs.rs/mio/1.2.4/mio/net/struct.TcpStream.html).
- Each queued frame is flushed immediately when writable. The single reusable
  TURN output slot preserves partial writes. The 32 KiB rustls buffer is a bound,
  not a fill target; ciphertext is drained in the same pass. Work budgets yield
  without sleeping, preventing receive bursts or writes from starving input.
- Existing protocol retransmission/acknowledgement tuning and the 240-second TURN
  renewal are retained. TCP loss still stalls the client-to-relay byte stream;
  UDP remains first choice. There is no claim of the globally lowest latency.
- Setup trust loading/DNS happens outside live media processing. Cancellation,
  certificate validation, bounded backpressure and teardown remain in place.
- The unused host encoder bitrate controller, experimental strategies and their
  host-only tests have been removed. Client retransmission levels are unchanged.
  Required protocol, framing, allocation-reuse, cancellation, TLS and WebRTC
  regression tests remain; they are exercised by CI and excluded from the normal
  executable. Optional diagnostic commands, fixtures, rcgen and diagnostic-only
  compiler support remain behind test/diagnostics build configuration.
- Direct dependencies were checked against their use sites and feature gates:
  rustls and trust stores implement TLS; Windows/Linux native IO,
  protocol, cryptography and vendor decoder dependencies serve retained features.
  No new dependency or unneeded SDK/C ABI, host encoder, browser transport or
  standalone vendor comparison tool is introduced by this audit.
  The unused lowlat-net-to-lowlat-crypto dependency edge was removed; required
  client cryptography is retained. A crate lint checks for unused direct net
  dependencies during compilation.
- Native packaging rejects diagnostic markers and synthetic video in the normal
  executable and checks runtime imports. The original Parsec WASM SHA-256 remains
  `d663dd96df477c65479fb93eb88756c7fcafff581cc93be563625cd195a4b4a6`.

Operational failures now retain only static setup stages, IO error kinds and
numeric OS codes, including synchronous carrier creation and asynchronous
connect/TLS/transport failures. Error strings, endpoints and credentials are
not copied into the application log. Synchronous carrier failures can advance
to another configured route just like worker-side transport failures.
Nonblocking connect follows Mio's documented pending-peer handling; TCP_NODELAY
is set after connectivity is confirmed, before TLS/application data is sent.

On 2026-10-16 the [Windows/Linux transport validation](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/38945891934)
passed, including 283 retained protocol tests, application-level direct/UDP/TCP
connectivity and all three 65-second authenticated coturn renewal cases. The
stream cases blocked the client's UDP source port. No measured real-video
latency or public-provider compatibility is inferred from these synthetic tests.

## Native TCP wait (2026-10-17)

The optional BUD stream carrier now uses the shell's native platform wake:
Linux polls the TCP fd beside eventfd; Windows associates the overlapped TCP
socket with that wake's IOCP. ConnectEx, WSARecv and WSASend have stable owned
operation storage. Synchronous completions can bypass the port when the provider
supports it; the ordinary completion mode is also tested. Cancellation drains
posted completions before releasing buffers. No additional I/O thread is added.

Windows copies outgoing network bytes into one reusable 32 KiB buffer because
an overlapped send must outlive the caller's borrowed slice. TLS ciphertext and
partial native sends count as pending until drained. Physical send completion,
not acceptance into that storage, updates write progress. The original frame age
and all setup/write deadlines are retained. This has no decoded-frame, GPU or
presentation impact. No throughput or latency improvement is claimed without
measurement. Mio's direct net dependency is removed, although Tokio still uses
Mio elsewhere in the application. Rustls remains the TLS provider.

See [native TLS feasibility](native-tls-investigation.md) for the separate
provider investigation and its buffering, trust and DTLS limits.
