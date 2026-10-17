# Local TURN stream transport integration

Base: crates.io `webrtc-ice` 0.14.0 (MIT OR Apache-2.0). Original licenses and
upstream sources remain in this directory. With transport preference disabled,
the original stream integration does not change ICE pair priorities, nomination
or candidate filtering. The application defaults to `all`, with an explicit
relay-only setting available to the user.

- `src/turn_stream.rs`: implements the existing `webrtc-util::Conn` contract
  over Tokio TCP and rustls TLS. RFC 8656 sections 7 and 11 define the STUN and
  ChannelData framing and stream padding; there is no additional RFC 4571
  length prefix. Fragmented reads, coalesced messages, serialized writes and
  terminal framing failures are handled explicitly. Close interrupts a pending
  read and shuts down the stream.
- `src/agent/agent_gather.rs`: allows `turn:...?transport=tcp` and
  `turns:...?transport=tcp` alongside the unchanged UDP path. The allocation
  still requests UDP toward the peer, exactly as defined by TURN. ICE continues
  to advertise a UDP relay candidate. RFC 6062 TCP allocations and ICE-TCP are
  different features and are not implemented by this patch.
- TLS validates hostname and certificate chain with rustls, public trust roots
  and OS-trusted roots. The legacy Parsec RSA compatibility option affects DTLS
  only and cannot weaken TURN TLS. TLS establishment is limited to ten seconds.
- Stream transport ownership follows allocation lifetime, including failed
  setup and explicit candidate shutdown. The already-connected server address
  is passed to the upstream TURN client to avoid a second round-robin DNS
  lookup choosing a different endpoint.
- The companion `turn-0.11.0` patch selects reliable STUN transactions for TCP
  and TLS: no UDP retransmissions, a 39.5-second transaction timeout, and immediate
  pending-request failure on transport closure (RFC 8489 section 6.2.2).
- `src/lib.rs` exports the transport for isolated regression tests.
- Measured stream buffering: plain TCP uses one bounded reusable receive
  buffer (64 KiB plus TURN header allowance), returning each available frame
  without waiting to fill it. TLS retains direct reads into the caller because
  rustls already buffers records. Padded ChannelData uses reusable storage to
  write the message and zero padding together; aligned messages and STUN keep
  their direct write. No batching timer, send queue or dependency is added.
  See `docs/turn-buffer-comparison.md` for the measurements and their limits.
- Read-only native telemetry: relay candidates retain their allocation URL (without
  credentials) and actual client-to-server transport. Authenticated, transaction-
  and source-matched STUN success responses update RTT and response counters for
  both ICE roles. No extra probes, candidate changes or nomination changes are
  introduced. Host/remote candidates do not fabricate relay transport information.
  The coturn fixture verifies selected-pair RTT and UDP/TCP/TLS provenance.

Limitations: relay gathering currently follows upstream's IPv4 allocation
path. TLS uses rustls; OS roots are imported, but this is not a Windows Schannel
connection and does not reproduce every enterprise trust policy. No HTTP proxy
or CONNECT tunneling is added. `stuns:` discovery remains unsupported.

Tests: `src-native/tests/turn_stream.rs` verifies framing, padding, malformed
frames, shutdown and untrusted TLS rejection. `tests/turn-integration` uses a
separate loopback coturn server and a temporary trusted CA to exchange real
DTLS/SCTP messages over UDP, TCP and TLS, and reject hostname mismatch. It is
never embedded in the client executable.

References: [TURN RFC 8656](https://www.rfc-editor.org/rfc/rfc8656),
[STUN RFC 8489](https://www.rfc-editor.org/rfc/rfc8489),
[TURN URI RFC 7065](https://www.rfc-editor.org/rfc/rfc7065).
# Opt-in TURN transport preference

`AgentConfig::prefer_turn_udp` defaults to false. Enabled by the native app, it
stages TURN gathering as UDP first, TLS second and plain TCP last. Each present
preferred tier receives a three-second connectivity head start. Delayed fallbacks
stop after ICE connects, fails or closes. An unanswered allocation does not block
the next tier. RFC 8445 relay local preferences rank these transports; direct
candidate priorities and remote nomination authority remain unchanged.
This bounded policy does not prove that a slower preferred route is impossible.
See `docs/native-turn-and-hotkeys.md` and the isolated coturn trickle-ICE fixture.


## Full discovery, relay-only connectivity extension

`SettingEngine::set_relay_only_connectivity(true)` (webrtc) propagates to
`AgentConfig::relay_only_connectivity` (webrtc-ice). This locally maintained API
is not provided by upstream and is not equivalent to W3C `iceTransportPolicy:
relay`: gathering/signaling retains real host and srflx addresses under policy
All, so it makes no address-privacy promise. The default is false.

The agent permits only local relay candidates in pair creation, pair selection,
outbound connectivity checks, inbound connectivity checks and inbound non-STUN
traffic. Gathering is unchanged. The restriction is installed at construction,
not after a direct connection has opened. If relay is unavailable it fails closed;
there is no fallback to host, srflx or peer-reflexive local data paths. DTLS
fingerprint verification and STUN authentication are unchanged. Separate coturn
fixtures cover full discovery with a relay-selected SCTP path and a usable direct
path that must remain disconnected when the restriction is enabled without TURN.
