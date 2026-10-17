# Windows TLS comparison

Standalone benchmark, never linked or packaged into ParsecWebTurn. Run the
optimized executable on the intended Windows client hardware, with the client
and other heavy workloads closed:

```
pwt-tls-comparison.exe tls-comparison.json
```

No administrator access, certificate installation, production profile, remote
server, credentials, media or GPU access is needed. An ephemeral CA and leaf are
generated in memory; both clients explicitly trust only the fixture CA.

Both providers use one identical rustls loopback server, TCP_NODELAY and a
single allowed AES-128-GCM cipher for each requested protocol. TLS 1.2 and 1.3
are tested separately; the server reports the actual negotiated version/cipher.
Resumption is disabled at the server. One warmup pair is discarded, then seven
paired trials alternate provider order. Every received payload is verified.

- Handshake duration is separate from steady-state measurements.
- 1,000 256-byte echo exchanges report p50/p95/p99 round-trip microseconds.
- Each direction transfers 64 MiB in 16 KiB chunks and reports MiB/s.
- Client-thread CPU time covers the throughput phase only, using Windows thread
  kernel/user counters. It excludes the identical server thread. Quantization
  and scheduling still require repeated trials.

Use matched local runs to decide. A repeatable improvement of at least 10% in
stream CPU cost or throughput, without worse control-message tail latency, is a
screening threshold, not proof of an end-to-end streaming improvement. Faster
handshakes alone are not sufficient reason to replace a steady-state media TLS
provider. Check absolute CPU cost at realistic stream bitrates as well.

The first test measures TLS provider behavior and wrappers. It does not validate
nonblocking IOCP backpressure, partial-write retry, cancellation, certificate
policy parity, TURN compatibility or application binary size. A migration still
needs those acceptance tests. GitHub runner results establish test operation,
not performance on the user's hardware. No production provider is changed.
