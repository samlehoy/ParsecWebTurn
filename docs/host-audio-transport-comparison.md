# Host packet timing versus completed audio messages

The stock browser and native Rust adapter both stutter with the tested host;
the stock native Parsec client was reported continuous. Host echo cancellation
off did not resolve it. The shared suspect is the host's WebRTC/SCTP path,
including its source pacing, transport, reassembly and ordered delivery.
This is a hypothesis, not a demonstrated fault in WebRTC itself.

## Historical capture and interpretation

The temporary host capture script and client `--trace-io` sampler have been
removed. The observations below describe the previous investigation, not
commands or instrumentation available in current builds.

### What the two boundaries can establish

- A gap in all outgoing stream datagrams at the host NIC points to something
  before or at that boundary, rather than the Rust audio decoder.
- Outgoing datagrams during an audio-message gap do not prove that the host
  sent audio: these same DTLS packets can contain video, control or SCTP ACKs.
- DTLS encrypts SCTP headers and audio. Packet size must not be used as a
  definitive audio identifier or a codec-independent sequence number.
- A second NIC capture on the client can match DTLS epoch/record sequence
  identifiers for the same direct session to separate transit from host NIC
  output. An audio-specific host send/reassembly comparison additionally needs
  supported plaintext SCTP telemetry; the stock proprietary host does not
  expose that through the inspected logs/configuration.
- Schema 5 IO samples have an absolute Unix-time anchor. Schema 4 has elapsed
  time only. Without a verified shared time anchor, compare distributions and
  bounds; do not claim precise temporal correlation or one-way latency.

No host binary, encryption, media framing, channel reliability, or scheduling
policy is modified for this recording.

## 2026-10-12 observations and limitations

The user clarified that the ten-minute schema-4 client trace and host capture
were a Cloudflare TURN trial. Audio was substantially better (not perfect)
there; switching back to direct communication over the Cloudflare One VPN
reintroduced the severe symptom. The VPN runs on the remote client and on the
host-side router. The local Windows host itself has no active VPN adapter.
Consequently its 1500-byte Ethernet MTU does not establish the tunnel's MTU.
The TURN-vs-VPN distinction is user-reported; schema-4 counters alone do not
verify the selected ICE route.

The client trace finishes with zero Opus decode errors and zero decoder packet
drops; cumulative maxima are 0.0645 ms handoff, 3.2587 ms decode queue wait and
4.1261 ms decode execution. The active output records 60 underruns and 107520
discarded PCM frames under the since-withdrawn experimental playback policy.
Source pauses/reconnections are present, so its 5968 ms lifetime arrival gap
must not be labeled an audible network interruption without corroboration.

The host NIC capture ran 14:24:29–14:25:29 UTC, with 1219 packet appearances,
including duplicated observations at two components. The first DTLS record
headers contain 658 unique outbound and 223 unique inbound epoch/sequence pairs.
The sequence spans are much larger (94497 and 38690). Capturing only a subset of
record headers at these observation points does not establish network loss;
coalescing, capture coverage and record layout must be accounted for. Therefore
the apparent multi-second gaps in this capture are **not accepted as proven
host-send gaps**. The schema-4 file also lacks an absolute clock anchor: its
copied file timestamp must not be used to assert exact host/client correlation.
The comparison is inconclusive at the audio-send boundary.

The route comparison now makes the Cloudflare One tunnel a concrete research
lead. Cloudflare documents [PMTUD and WebRTC MTU requirements](https://developers.cloudflare.com/cloudflare-one/team-and-resources/devices/cloudflare-one-client/deployment/mdm-deployment/path-mtu-discovery/):
the current guide warns of degraded WebRTC performance below a 1361-byte network
path MTU and provides `warp-cli settings` / `warp-diag` checks. Its PMTUD feature
requires MASQUE and current clients enable it by default. This is vendor guidance,
not proof of this session's failure or a universal minimum for every SCTP packet.
Do not change the application's packet sizing, router MTU or VPN protocol
without verifying the actual tunnel type and path. A router cloudflared connector
and a WARP/Mesh node are different configurations and must be identified first.
