# Windows TCP/TLS QoS investigation — 2026-10-26

## Result

A separate Windows probe successfully attached an already connected, dual-stack
TCP socket to a qWAVE non-adaptive AudioVideo flow. `QOSQueryPacketPriority`
returned DSCP **40** and L2 **0**. A certificate-validated TLS 1.2 HTTPS HEAD
request to `example.com` returned HTTP 200 both with and without the flow.
Create/add/query took **1.9847 ms in that single marked trial**, at connection
setup, not per packet. This is an API/compatibility observation, not a latency
benchmark or a packet-capture proof of DSCP on the wire.

IPv4-mapped and IPv6 loopback flows were refused with error 1168; TCP payload
still arrived. Loopback therefore cannot establish scheduling benefit. An
initial Microsoft HTTPS target timed out in both control and marked trials;
the successful example.com control/marked pair above is the usable result.

No congestion, input-latency or stream A/B comparison has established a benefit.
The exploratory script/results are outside the application source and are not
compiled into or shipped with EvoParseq. No production QoS option is added yet.

## Fit with the current client

- BUD already has a dynamically loaded qWAVE helper for its UDP paths. Its UDP
  connect/mark/disconnect sequence must **not** be reused on established TCP.
- BUD TCP/TLS shares one overlapped TCP socket. A future implementation can
  reuse the API loader/lifetime at successful ConnectEx completion, without
  modifying the read/write buffers, completion port, TLS records, or media path.
- WebRTC TURN TCP/TLS has separate Tokio/socket ownership. The BUD helper cannot
  automatically cover it; its socket lifetime needs its own explicit adapter.
- A client marks its **outgoing** traffic only: input, control and TCP ACKs may
  benefit under a local upstream bottleneck. The host/relay's incoming video is
  not automatically marked by this request. One multiplexed TCP connection also
  cannot assign independent per-message priority to video/audio/input records.
- Use `QOSTrafficTypeAudioVideo` with `QOS_NON_ADAPTIVE_FLOW`, no rate shaping,
  path probes or arbitrary DSCP override. Failure must leave the connection
  usable, with no repeated per-packet setup or mandatory service requirement.
- `QOSSetOutgoingDSCPValue` has additional privilege requirements and is not
  needed for this approach. No system policy, registry, certificate verification,
  congestion algorithm or transport selection was changed for the probe.
- Configured priority is not an end-to-end guarantee. Network policy/routers
  may ignore or rewrite it; L2 marking was zero in this trial.

## Before enabling it in production

Compare disabled/enabled trials on the same real TURN TCP/TLS route, alternating
order, both idle and with controlled local upload contention. Measure input RTT,
p50/p95/p99 stalls and existing stream timing/drop counters. Capture only the
isolated test flow to verify outgoing DSCP; measure downstream congestion
separately. Keep it disabled until this establishes a worthwhile benefit and
verify refusal/service-unavailable cleanup without losing connectivity.

## Primary API references

- [QOSAddSocketToFlow](https://learn.microsoft.com/en-us/windows/win32/api/qos2/nf-qos2-qosaddsockettoflow)
- [QOS traffic types](https://learn.microsoft.com/en-us/windows/win32/api/qos2/ne-qos2-qos_traffic_type)
- [QOSQueryFlow](https://learn.microsoft.com/en-us/windows/win32/api/qos2/nf-qos2-qosqueryflow)
- [QOS_PACKET_PRIORITY](https://learn.microsoft.com/en-us/windows/win32/api/qos2/ns-qos2-qos_packet_priority)
- [QOSSetFlow and DSCP privileges](https://learn.microsoft.com/en-us/windows/win32/api/qos2/nf-qos2-qossetflow)
