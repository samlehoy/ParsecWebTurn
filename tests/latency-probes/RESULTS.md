# Screening results, 2026-10-14

Probe revision: `507c751`. The normal client changes are in `75c85fa`.
These results do not introduce any further production transport/decoder changes.

## UDP bursts

[Windows/Linux measurement run](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/38556223345).
Values below are the median of five trials' **p95 UDP receive latencies**, in
microseconds. Each trial discards its first 16 bursts as warmup and alternates
baseline/candidate ordering. There were no missing or duplicate datagrams in
either mode on either OS.

| OS | Packets per burst | Existing `recv_from().await` | Candidate drain of 32 |
| --- | ---: | ---: | ---: |
| Windows | 16 | 18.2 us | 18.8 us |
| Windows | 64 | 9.9 us | 9.7 us |
| Windows | 256 | 6.9 us | 7.4 us |
| Linux | 16 | 30.07 us | 22.01 us |
| Linux | 64 | 24.93 us | 21.76 us |
| Linux | 256 | 17.39 us | 21.70 us |

The candidate is not consistently better. On Windows the receive differences
are sub-microsecond; the control proxy improved by only 1.3-3.8 us in these
aggregates, and competing-timer p95 also regressed in the 64-packet case. On Linux,
large bursts worsened receive and control-proxy p95 despite improved timer
fairness. This is not sufficient evidence to change the live WebRTC transport.
No full WebRTC experiment or production batching change was adopted.

This is a loopback socket screening test, not ICE/DTLS/SCTP/TURN or internet
latency. It cannot predict remote-host or notebook performance. See README for
the exact payload, buffer, sender, timer and scheduling conditions.

## H.264 decoder output availability

The Windows artifact from the same run was executed locally on **AMD Radeon 780M
Graphics**, using Media Foundation and actual D3D11 decoder surfaces. Ten trials
decoded all **800 pictures**, without decoded-pixel mapping/readback. Both property
values were accepted. Each trial excludes the first 16 pictures from its latency
distribution; the table contains the median of five trials' reported statistics.

| H.264 low-latency property | p50 output availability | p95 output availability | Pending before final drain | Maximum later inputs before output |
| --- | ---: | ---: | ---: | ---: |
| Disabled baseline | 216.536 ms | 283.226 ms | 16 in every trial | 17 in every trial |
| Enabled, as already requested by our client | 0.260 ms | 0.345 ms | 0 in every trial | 0 in every trial |

This confirms that the existing H.264 low-latency request avoids a large output
hold on this GPU/decoder with the synthetic no-B-picture fixture. **It is not a
new client speedup:** the production client already requests the enabled value.
No further decoder change follows from this result. The baseline intentionally
disables that property and does not describe the shipped client's latency.

The interval starts just before `ProcessInput` and ends at the matching
`ProcessOutput` sample. It includes any wait for later input pictures. It excludes
network reception, the compressed-frame queue, display presentation and GPU
completion. The final drain only verifies complete delivery of the test's last
samples. No SPS/VUI rewriting or HEVC timing comparison was performed. Results
on other GPU/driver combinations remain unmeasured.
