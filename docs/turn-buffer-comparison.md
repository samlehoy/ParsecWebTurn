# Tokio TURN stream buffering comparison — 2026-10-17

Decision: retain Tokio and rustls. The measurement supports targeted buffering,
not replacing WebRTC's async runtime with the BUD native poller. The measurement
itself left the production client unchanged; the subsequent implementation is
recorded below.

The supported candidates are a reusable receive buffer for **plain TURN/TCP**
and coalescing a ChannelData message with its transport padding in one write.
TLS receive buffering did not demonstrate a useful speedup. Sending multiple
already-ready messages together has a larger synthetic benefit, but requires
an explicit bounded producer/queue contract that WebRTC's current Conn send
interface does not provide; it is not a drop-in optimization.

## Subsequent production implementation

The WebRTC TURN carrier now applies Buffered RX only to plain TCP. It retains
the baseline direct receive path on TLS. Complete buffered messages are copied
into Conn's caller-owned output; only an incomplete tail needs compaction.
The receive capacity includes the largest legal STUN header/body and maximum
ChannelData padding, so the optimization does not narrow accepted message sizes.

TCP and TLS both write padded ChannelData as one framed write from fixed reused
storage. Unpadded messages, including STUN and aligned ChannelData, retain direct
writes without a staging copy. The original send lock covers the complete
write/flush, including partial progress; supplied padding is normalized to zero.
This is the Frame TX policy, with direct TLS RX, rather than Ready TX.

No multi-message queue, coalescing delay, timer change, new thread or dependency
is introduced. ICE gathering/selection, UDP, DTLS/SCTP, BUD, WASM, audio and GPU
paths are untouched. Regression fixtures cover fragmented/coalesced frames,
buffer-boundary tails, maximum STUN/ChannelData lengths, every padding length,
truncated EOF, terminal errors, cancellation of a partial TCP receive, shutdown
and certificate rejection. The real coturn fixture exercises DTLS/SCTP over
UDP/TCP/TLS independently of the synthetic benchmark.

## Additional lowlat WebRTC review

Reviewed current main/client head on 2026-10-17:
[`1d2bb76d3cab31ab73dbd88ca07d2e071683a7bb`](https://github.com/nomi-san/lowlat/tree/1d2bb76d3cab31ab73dbd88ca07d2e071683a7bb),
alongside the already reviewed `feat/relay-tcp` head `b174dc6`.

- The [browser example transport](https://github.com/nomi-san/lowlat/blob/1d2bb76d3cab31ab73dbd88ca07d2e071683a7bb/examples/web-client/src/transport.ts)
  uses the browser's RTCPeerConnection, configures a STUN URL, and forwards only
  host/srflx candidates; it discards relay candidates. It contains no custom
  TURN/TCP or TLS IO implementation to reuse. Browser WebRTC capability must not
  be confused with features this example actually configures.
- The [native web module](https://github.com/nomi-san/lowlat/blob/1d2bb76d3cab31ab73dbd88ca07d2e071683a7bb/crates/net/src/web/mod.rs)
  serves browser guests on the host side, using sans-IO DTLS/SCTP state machines.
  Its [dependencies](https://github.com/nomi-san/lowlat/blob/1d2bb76d3cab31ab73dbd88ca07d2e071683a7bb/crates/net/Cargo.toml)
  are dimpl and dcsctp. This is not the native client's browser/WebRTC transport
  and does not supply our Tokio TURN/TLS adapter.
- [Connectivity section 7.4](https://github.com/nomi-san/lowlat/blob/1d2bb76d3cab31ab73dbd88ca07d2e071683a7bb/docs/03-connectivity.md)
  implements native client relay over UDP/TCP and explicitly does not offer TLS.
  DTLS and TLS are different layers; having browser DTLS is not TURN/TLS support.
- Useful general ideas remain: externally driven state machines, bounded queues,
  explicit backpressure and separate control/audio/video priorities. These do not
  establish a lower-latency replacement for our tested stack. In particular,
  changing SCTP priorities, ordering/reliability or adopting a new DTLS/SCTP stack
  is outside this buffering change and would need independent interoperability
  tests. No lowlat browser transport or new protocol dependency is imported.

## Method and reproducibility

- AMD Ryzen 7 8845HS, 16 logical processors, Windows 10.0.26300.0.
- [Isolated source](https://github.com/vampywiz17/ParsecWebTurn/tree/effebfc3630182c7cae32f959cac7828ca0389aa/tools/turn-buffer-comparison),
  revision `effebfc3630182c7cae32f959cac7828ca0389aa`.
- [Windows/Linux fixture validation](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/39057672544)
  passed. CI timing is not used as evidence of local performance.
- [Raw local trials, call counts, medians and paired comparisons](measurements/turn-buffer-comparison-2026-10-17.json).
- The baseline compiles the actual production
  `src-native/vendor/webrtc-ice-0.14.0/src/turn_stream.rs`, exposing only a
  constructor for the isolated fixture. SHA-256:
  `90d4ff86a3517054375b88820f0689bb2147eb40b3435b51eed748817880bf33`.
- Same Tokio 1.53.2 and rustls 0.23.46 as the client. Release opt-level=s,
  thin LTO, one codegen unit, static CRT. One current-thread Tokio client
  runtime and a common echo/push server on a separate OS thread.
- Loopback TCP_NODELAY, plain TCP and verified TLS 1.3 separately. Explicit
  ephemeral fixture certificate trust and localhost verification, no installed
  certificates, production credentials, external hosts or TLS resumption.
- Two complete local runs, six measured repetitions per policy/workload/carrier
  in each run, after discarding a warmup for each. Policy order rotates each
  repetition. Thus each performance cell contains 12 trials.
- Uninstrumented runs measure timing. A separate local run wraps AsyncRead and
  AsyncWrite to count polls, successful calls and bytes, verifying equal
  plaintext byte counts. These calls are **not OS syscall counts**.

The generated candidate preserves the baseline locking, close notification,
TLS setup, parser and original per-message send path. Its experimental changes
exist only in the harness. The one-off harness and CI workflow were removed
after the investigation; their historical source and raw evidence remain.

## Four policies

| Name | Receive | Send |
| --- | --- | --- |
| Current | Read header, body and padding separately into caller storage | Body and padding separately, flush per message |
| Buffered RX | Reused 64 KiB buffer, copy each complete frame to caller | Current |
| Frame TX | Buffered RX | Reused 32 KiB storage, body and padding together, flush per message |
| Ready TX | Buffered RX | Same storage, groups only messages already supplied, flush immediately at capacity/end |

The candidates adapt BUD's buffering **policy to Tokio**; this is not a timing
comparison of BUD's IOCP/poll driver versus Tokio. Unlike BUD's borrowed frame
slice, Conn::recv requires the message in caller storage. Buffered RX therefore
adds a full frame copy; it cannot inherit BUD's in-place zero-copy claim. The
transmit adapter also copies existing Conn input to staging. No decoded video
frame or GPU resource is involved.

## Workloads

- Single echo: 2,000 sequential 260-byte messages (including TURN header).
  One ready message at a time, no padding or fill timer.
- Padded duplex burst: 512 rounds of 64 already-ready 1,023-byte ChannelData
  messages, each with one padding byte: 32 MiB in each direction.
- Aligned duplex burst: same, but 1,204-byte messages with no padding:
  37.625 MiB each direction. This separates batching from padding coalescing.
- Receive-only burst: server pushes the corresponding 32/37.625 MiB sequences
  after a startup barrier; client only drains and verifies them.
- Separate correctness workload: the common server writes only three bytes
  at a time. Every variant reconstructs and verifies the messages over both
  TCP and TLS. Its timings are excluded from the performance tables.

Every received logical message and byte count is verified. Echo server reads
whatever is available and echoes it, without waiting for a complete group.
The sender never waits for more messages. No producer thread or batching timer
is introduced. Timing excludes connect/handshake and ends after all replies
or pushed frames have been consumed.

## Local results

Cells are medians of per-trial results; percentile cells are medians of the
individual trials' percentiles, not pooled percentiles. Throughput in duplex
tests counts one direction, not the sum of both directions.

### Receive-only capacity, MiB/s

| Carrier / message | Current | Buffered RX |
| --- | ---: | ---: |
| TCP / padded | 38.31 | 742.55 |
| TCP / aligned | 70.48 | 868.23 |
| TLS 1.3 / padded | 667.98 | 657.46 |
| TLS 1.3 / aligned | 639.09 | 645.77 |

TCP burst receive capacity improves consistently here; paired padded trials
have an 18.62x median throughput ratio. This is an unpaced synthetic stress
result, **not** an 18x live-client speedup or network-bandwidth increase.
Baseline TCP also exhibits millisecond-scale group stalls in this stress case;
those group timings do not establish real-session input/video delay.

TLS already buffers records internally: the additional frame copy and buffer
do not yield a substantial receive-throughput improvement. The padded paired
throughput ratio is 0.99, while aligned medians differ by about one percent.

### Duplex burst completion p95, microseconds

Each sample times sending and consuming an entire ready group of 64 messages,
not one frame's network flight time.

| Carrier / message | Current | Buffered RX | Frame TX | Ready TX |
| --- | ---: | ---: | ---: | ---: |
| TCP / padded | 2014.20 | 1871.90 | 937.60 | 262.00 |
| TCP / aligned | 1092.20 | 960.05 | 948.60 | 244.95 |
| TLS 1.3 / padded | 1753.95 | 1775.85 | 982.90 | 299.00 |
| TLS 1.3 / aligned | 993.90 | 996.60 | 981.50 | 322.15 |

Coalescing padding helps padded messages, but adds little for already-aligned
TLS messages. Ready TX helps both sizes, showing that the batching benefit
is not solely an artifact of a separate one-byte padding write.

### Single-message echo RTT p95, microseconds

| Carrier | Current | Buffered RX | Frame TX | Ready TX |
| --- | ---: | ---: | ---: | ---: |
| TCP | 34.45 | 33.50 | 35.90 | 33.10 |
| TLS 1.3 | 68.70 | 69.80 | 68.15 | 66.90 |

These small changes do not establish a substantial single-message latency
improvement. Within paired TLS trials, the three policies' median p95 ratios
are 1.00, 1.02 and 1.01 respectively. Ratios of separately summarized medians
need not equal the median within-pair ratio.

### Separate call-count run

For 32 MiB of padded traffic (32,768 messages):

- Plain receive: 98,354 successful underlying reads with Current versus 525
  with Buffered RX. Fragmented reads can make counts exceed three per message.
- Duplex plaintext writes: Current/Buffered RX 65,536; Frame TX 32,768;
  Ready TX 1,024. These are calls at the measured stream layer.
- TLS Ready TX makes 2,048 successful underlying socket writes in this run,
  despite 1,024 plaintext writes, because TLS splits data into records.
- TLS receive: Current performs 98,304 plaintext reads, but only 3,961
  successful socket-layer reads, demonstrating rustls' existing buffering.

## Integration implications and limits

1. Keep Tokio and rustls. No native-runtime migration, dependency removal or
   overall Parsec CPU/latency benefit was established.
2. Consider reused receive buffering for plain TURN/TCP. For TLS, retain the
   current direct copy into the caller unless another measurement justifies
   a buffer on top of rustls.
3. Consider a single framed write for message plus padding, without a timer.
   Reuse bounded storage, retain send ordering, cancellation safety, certificate
   validation and terminal handling for partial failures.
4. A send-many implementation needs a bounded queue and wake/drain policy.
   Do not delay isolated input/audio messages to create these synthetic groups,
   acknowledge unwritten data as delivered, grow a backlog, or silently change
   Conn cancellation semantics. Measure again at the actual WebRTC producer.

The test deliberately excludes real TURN allocation/renewal, ICE, DTLS, SCTP,
congestion, packet loss, WAN shaping, decode and display. Real WebRTC sessions
are receive-heavy and usually far below these saturation rates. No benefit to
direct UDP or TURN/UDP follows from this test; TCP head-of-line blocking and
SCTP reliability/ordering remain. TLS 1.2 timing, CPU consumption, allocations,
power and actual end-to-end Parsec latency were not measured.
