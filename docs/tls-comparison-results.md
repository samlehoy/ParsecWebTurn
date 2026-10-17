# TLS provider comparison — 2026-10-17

Decision: retain rustls. Native-tls/Schannel did not demonstrate a substantial
overall low-latency advantage in this first local comparison. No production TLS,
DTLS certificate verifier, WebRTC policy, decoder or proprietary WASM was replaced.

## Setup

- AMD Ryzen 7 8845HS, 16 logical processors, Windows 10.0.26300.0.
- rustls 0.23.46/ring versus native-tls 0.2.18/Windows Schannel.
- [Isolated source and method](../tools/tls-comparison/README.md), source commit
  `f9d433216bb2255f419abeea748540fce78ce016`.
- Client-matching release profile: opt-level=s, thin LTO, one codegen unit,
  static CRT. No production profile, credentials or installed test certificate.
- One identical rustls loopback server; TCP_NODELAY; TLS 1.2/1.3 separately,
  AES-128-GCM only, no server resumption. Negotiation was checked by the server.
- Two complete local runs; each discarded one warmup pair per protocol, then
  alternated provider order for seven pairs. Fourteen measured trials per
  provider/version; 56 trials total. Each trial had 1,000 256-byte echo exchanges
  and 64 MiB in each direction, with payload verification.
- [Raw results and paired summary](measurements/tls-comparison-2026-10-17.json).
  Earlier exploratory opt-level=3 runs were not used in this final table.

The [measurement CI](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/39021460872)
passed. Its timing is not used as local performance evidence.

## Local medians

Each cell summarizes fourteen trials; p95/p99 columns are medians of the
individual trials' percentiles, not pooled percentiles.

| Version / provider | Ping p50 µs | Ping p95 µs | Ping p99 µs | Upload MiB/s | Download MiB/s | Receive CPU ms/MiB | Handshake ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| TLS 1.2 / rustls | 18.05 | 25.20 | 41.90 | 563.5 | 987.9 | 0.977 | 0.583 |
| TLS 1.2 / native-tls | 18.25 | 27.60 | 47.95 | 641.8 | 924.9 | 0.732 | 2.342 |
| TLS 1.3 / rustls | 32.75 | 42.60 | 69.85 | 367.1 | 515.8 | 1.221 | 0.658 |
| TLS 1.3 / native-tls | 32.40 | 43.95 | 72.10 | 380.2 | 486.5 | 0.977 | 3.852 |

Paired comparisons are calculated within each alternating pair, so their median
ratios need not equal ratios of the table's independently calculated medians:

| Native-tls relative to rustls | TLS 1.2 | TLS 1.3 |
| --- | ---: | ---: |
| Median receive CPU reduction | 25.0% | 18.3% |
| Median receive CPU saved | 0.244 ms/MiB | 0.244 ms/MiB |
| Pairs with lower receive CPU | 10/14 | 9/14 |
| Median download throughput change | -4.7% | -10.5% |
| Median ping p95 change | +2.2 µs | +0.9 µs |

## Interpretation and limits

There is an indication of lower receive-thread CPU cost with native-tls. It is
not a consistent latency or receive-throughput improvement. The median absolute
CPU saving is small: linearly scaling 0.244 ms/MiB to a 50 Mbit/s receive stream
would be about 1.46 ms of one thread's CPU time per second (0.15 percentage points
of one core). That is an extrapolation, not measured Parsec CPU usage.

GetThreadTimes has coarse observed accounting increments; short per-direction
CPU measurements are quantized. Treat the percentages as screening estimates,
not a precise crypto-engine speed ratio. Background scheduling, server limits
and loopback copies affect throughput and echo timing. The results compare the
two provider/wrapper paths, not isolated AES or complete client performance.

No app binary-size saving, WAN/TURN latency improvement, async IOCP backpressure,
certificate-policy equivalence or public relay compatibility was demonstrated
by this tool. Those would need separate validation before migration. Both
providers already exceed typical stream throughput by a wide margin here.
The measured differences do not justify a TLS migration for latency at present.

The previously authorized native TCP wait adaptation is independent: its
[Windows/Linux backend validation](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/39020712420)
passed, including native IOCP cancellation, TLS/framing regressions and coturn
UDP/TCP/TLS renewal. It retains rustls above native IO.
