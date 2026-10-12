# Host packet timing versus completed audio messages

The stock browser and native Rust adapter both stutter with the tested host;
the stock native Parsec client was reported continuous. Host echo cancellation
off did not resolve it. The shared suspect is the host's WebRTC/SCTP path,
including its source pacing, transport, reassembly and ordered delivery.
This is a hypothesis, not a demonstrated fault in WebRTC itself.

## Isolated host recording

Run `scripts/capture-host-network.ps1 -OutputDirectory <new-directory>` in an
administrator PowerShell while continuous sound plays and the remote client
runs with `--trace-io`. The Windows Packet Monitor driver requires administrator
rights; the application itself does not. The script finds the active parsecd
UDP ports, records 60 seconds at network adapters with 96-byte truncation, stops
its own capture and removes only its named filters. Existing filters abort the
test; an already-running capture must not be interrupted. Do not run concurrent
Packet Monitor sessions or change filters while this test runs.

Capture output contains local/network addresses and partial encrypted packet
data. Keep it local. Only anonymized timing/count summaries belong in research
notes; never publish the raw ETL, text or pcapng in the repository.

Microsoft documents [Packet Monitor](https://learn.microsoft.com/en-us/windows-server/networking/technologies/pktmon/pktmon-syntax)
and its [pcapng conversion limitations](https://learn.microsoft.com/en-us/windows-server/networking/technologies/pktmon/pktmon-pcapng-support).
Zero lost capture events is not proof that no network packets were lost.

## What the two boundaries can establish

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
