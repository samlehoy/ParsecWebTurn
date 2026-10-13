# Relay-only candidate delivery

Investigated 2026-10-13 using the unchanged, SHA-pinned 150-104a core.

## Observed failure

A user-provided comparison of two connections to the same host showed:

| Observation | Force TURN | Ordinary policy |
| --- | --- | --- |
| Local relay candidates | 3 | 1 |
| Local host / srflx candidates | 0 / 0 | 1 / 2 |
| Local and remote descriptions installed | Yes | Yes |
| Remote candidates submitted to ICE | 0 | 6 |
| Parsec sync received | No | Yes |
| Result | Establishment deadline after 30 seconds | ICE, DTLS and SCTP connected |

TURN allocations succeeded in the failing attempt. The trace does not establish
why the extra Parsec synchronization message was absent. It also does not expose
per-attempt candidate queue contents; submitted candidates are not the same as
received candidates. The aggregate original-core import counters included remote
candidate callbacks across both attempts.

The Rust adapter copied the original browser shim's additional sync gate. In a
relay-only session without that marker, the gate indefinitely withholds otherwise
valid remote candidates even after the remote description has been installed.
This behavior is independently reproduced by the isolated regression fixture.

## Candidate-delivery adjustment

For an effective `RTCIceTransportPolicy::Relay` configuration, deliver validated
remote candidates using standard trickle ICE once the remote description is
installed. The peer retains its relay-only policy: this does not enable direct
local candidates or silently fall back to direct routing.

Ordinary connections retain the original synchronization behavior. If a real
Parsec sync marker arrives, its existing acknowledgment remains unchanged. No
ICE address is fabricated, and a sync placeholder is never used as a peer.
Attempt identity checks, credential/fingerprint validation, candidate validation,
deduplication and queue limits remain in place. The WASM core is unchanged.

Reference: [W3C addIceCandidate](https://www.w3.org/TR/webrtc/#dom-peerconnection-addicecandidate).

## Verification

Unit tests cover missing remote descriptions, stale attempt identifiers,
duplicates and late synchronization. The coturn fixture uses the production
`CandidateGate` with no Parsec sync marker and verifies all three real DTLS/SCTP
data channels, bidirectional payloads and a selected local relay candidate.
This synthetic test does not execute a real Parsec host.

## Live retest result

On 2026-10-13 the user retested the dd28f3e client and reported that Force TURN
still did not connect. Removing the additional candidate gate is therefore not
a verified fix for the reported Cloudflare/Parsec failure. Do not list it as a
resolved issue in release notes. A fresh trace from the matching diagnostic
build is needed to distinguish absent remote candidates from connectivity
checks that run but fail. The earlier trace cannot establish the new failure
stage.


## Follow-up trace and outbound readiness

The matching dd28f3e diagnostic trace submitted on 2026-10-13 confirms that the
previous delivery adjustment worked: Force TURN submits six remote candidates,
up from zero, while retaining three local relay candidates and zero local
host/srflx candidates. Nevertheless ICE stays checking, DTLS stays new and the
30-second establishment deadline expires. The ordinary-policy attempt connects.
No peer sync marker arrives in the forced attempt.

The adapter previously emitted its own sync marker only as a delayed reply to
a peer marker. Inspection of the pinned original shim confirms this behavior:
`ka()` schedules `a.l("1.2.3.4",1234,true,false,false)` after a received sync.
The lowlat signaling reference explains the complementary host-side barrier:
a peer may withhold candidate processing until the client sends readiness, and
the marker belongs after real candidates. This is application signaling on top
of ICE, not an ICE requirement or a real address.

The next test client therefore queues its own readiness marker for relay-only
attempts after both real ICE gathering completion (`on_ice_candidate(None)`) and
installation of the remote description, provided at least one usable relay
candidate was gathered. It does not depend on a received peer marker. Exactly
one marker is queued per attempt; the existing delayed response remains for
ordinary attempts and is deduplicated if readiness was already queued. Ended
attempts cannot emit readiness and the existing bounded event queue is retained.

Diagnostics report `local_gathering_complete` and `local_sync_queued`. Queued is
not proof of signaling-server or host receipt. No TURN bypass, fake host/srflx
candidate, fingerprint relaxation, core modification or media-path change is
included. Unit tests cover ordering, prerequisites, duplicates, retired attempts
and queue overflow. The coturn integration uses the production readiness helper
and a peer candidate gate that withholds relay addresses until readiness, then
checks bidirectional data on all three DTLS/SCTP channels. Real Cloudflare/Parsec
compatibility still requires a live retest.

Reference: [lowlat signaling readiness](https://github.com/nomi-san/lowlat/blob/main/docs/04-signaling.md).


## Readiness client live retest

The user retested the 2649bc9 normal client on 2026-10-13 and reported that
forced TURN still fails both with and without the VPN. This is not a verified
resolution. The matching diagnostic package is available from successful CI
run 38369099018; its `local_gathering_complete` and `local_sync_queued` fields
are needed before drawing a conclusion about the updated readiness path.
The earlier dd28f3e trace does not contain these fields and must not be treated
as evidence of this new build's outbound readiness behavior.

`RTCIceTransportPolicy::Relay` is configured in the Rust WebRTC peer. It is not
passed as a force-relay option to the unchanged WASM core. The core does receive
the resulting compact relay addresses through its existing candidate events;
its host-side interoperability still needs investigation. Failure without the
VPN points away from a VPN-only explanation but does not establish which
endpoint, signaling step or relay connectivity check is responsible.


## Follow-up diagnostic trace

The next 2649bc9 trace confirms real gathering completion, local readiness
queued, three local relay candidates and six submitted remote candidates. The
peer sync is still absent and ICE remains checking until the deadline; DTLS
never starts. Queueing is not proof of delivery to the signaling server/host.
The ordinary attempt connects. The trace cannot distinguish an absent network
response from TURN permission/send failures or rejected ICE checks.

A separate diagnostics-only observation window now counts fixed, sanitized
pinned-library ICE/TURN log categories and snapshots public candidate-pair
states before close (500 ms collection bound). The window is process-scoped,
not a strict per-peer logger correlation. Check-attempt events are not proof of
NIC transmission. The selected candidate types and TURN transport are included
without IPs, candidate IDs, URLs, credentials or raw messages. Unimplemented
upstream request/packet counters stay null; unavailable logger/stats stay null.
No library log category controls production routing.

Cloudflare denies permissions for private peer addresses even without a VPN;
this is a possible diagnostic category, not an established cause of this
failure. Its FAQ documents the restriction:
https://developers.cloudflare.com/realtime/turn/faq/


## Full discovery with enforced relay connectivity

At the user's request, the next test separates gathering/signaling from the
allowed data path. The settings resolver still expresses Force TURN as a relay
request. The native worker translates this into `RTCIceTransportPolicy::All`
plus the project-owned `SettingEngine::set_relay_only_connectivity(true)`.
Actual host, srflx and relay candidates are gathered and signaled unchanged to
the pinned Parsec core. No candidate type or address is fabricated.

This is an explicit local Rust library extension, not the W3C `relay` policy and
not an upstream webrtc-rs API. It preserves standard ICE messages, authentication
and fingerprint checks. It intentionally does not hide LAN/public discovery
addresses from the peer. It enforces the user's requested TURN data path instead.
There is no connect-direct-then-switch interval: the restriction is set before
the peer is created. Candidate-pair creation, selected-pair assignment, outbound
ICE checks, inbound ICE checks and inbound non-STUN traffic all reject local
non-relay candidates. Normal STUN discovery traffic to configured servers still
works through the existing gathering implementation. If no relay works, the
connection fails closed. Default-off behavior is unchanged.

The regression fixture verifies actual host/srflx/relay discovery with coturn,
full candidate exchange, all three bidirectional DTLS/SCTP data channels and a
selected local relay. A separate negative fixture first establishes that the
host-only route works, then enables the restriction with no TURN server and
verifies that neither endpoint opens its data channel and the restricted peer
has no selected pair/checklist. The unchanged WASM and real Parsec host are not
executed by that fixture; a successful fixture is not proof of live compatibility.

Diagnostics distinguish `ice_gathering_policy: all`, `relay_only_policy: false`
(the literal W3C policy) and `relay_only_connectivity: true` (the project routing
restriction). Selected candidate types provide independent routing evidence.
The added observation counters remain exclusive to separate diagnostic builds.


## Full-discovery client live retest

On 2026-10-13 the user reported that the a514d4c normal client still does not
connect with Force TURN. Full discovery with restricted relay connectivity is
therefore not a verified resolution of this real-host failure. Its CI run
38374995044 passed, including both new routing fixtures, Windows tests/Clippy
and original-WASM probes; those fixtures do not establish Parsec host behavior.
The matching detailed diagnostic package is available and must be used to check
actual gathered candidate types, permission/check observations and the selected
path of the ordinary attempt. Do not assume that an ordinary connection selected
TURN solely because TURN was configured or a relay candidate was allocated.
No further production routing experiment is justified by the old trace alone.


## Detailed a514d4c diagnostic result

The matching user trace on 2026-10-13 contains a forced first attempt and an
ordinary second attempt (the user explicitly confirmed this ordering).

Forced: gathering All, relay-only connectivity enabled, 2 host + 2 srflx + 3
relay candidates, 6 remote candidates, peer sync received and local sync queued.
All 18 relay candidate pairs fail. The observation window has 144 binding-check
attempt events, 96 STUN send failures, 99 permission transaction events and 6
successful channel-bind events, but zero authenticated STUN success events or
responses. ICE stays checking until the 30-second deadline; DTLS stays new.
A ChannelBind success proves server-side setup, not arrival at the Parsec host.

Ordinary: 2 host + 2 srflx candidates, no gathered local relay candidate, a
selected host/host UDP pair, 2 authenticated responses and connected DTLS/SCTP.
This trace does not verify a working client-side Cloudflare relay path. It does
not contradict the user's deliberate Force-off second trial or establish that
all earlier ordinary sessions used the same route.

The accessible local Parsec host log for the matching time window shows a
failed connection at 11:38:07 with -6023 / -11004, and a connected SCTP peer at
11:38:36. The successful peer address is in the CGNAT 100.64/10 range; no full
address or account handle is retained here. The official Parsec documentation
categorizes the accompanying code as a possible UPnP availability issue, not a
unique diagnosis of this failure:
https://support.parsec.app/hc/en-us/articles/32381913636244-Error-Codes-6023-and-6024-Unable-To-Negotiate-A-Successful-Connection

Diagnostic limitation found during source review: relay send errors pass through
webrtc-util's `IoError` formatter, which prefixes `io error: `. The permission
subclassifier only matches a bare CreatePermission error string. Therefore the
zero permission_403/other category counts do not establish absence of permission
errors. The aggregate STUN send-failure count is valid; exact permission causes
remain unknown. A future diagnostic correction should recognize the pinned
wrapper and test the actual wrapped error, rather than infer a code from counts.

No new production routing change is justified by this trace alone. Remaining
questions concern the relay-to-host data path or handling/rejection of checks,
not missing discovery or synchronization. Inspecting the real packet exchange
or an independent TURN transport probe is needed to distinguish those cases.


## Single-endpoint forced relay (2026-10-13)

The user's next a514d4c diagnostic trial with a single TURN URL succeeded:
`relay_only_connectivity: true`, one local relay candidate, local gathering
complete and sync queued, six remote candidates, connected ICE/DTLS, all three
data channels open, and `video_rendered: true`. This snapshot was taken while
the worker was active; `ice_checks_before_close` is null, so it does not record
the selected pair's types or TURN transport. The routing restriction was active.
The user subsequently clarified that this successful test used ExpressTURN,
whereas the failed multi-URL trials used Cloudflare. Provider and endpoint count
therefore changed together. This proves a forced relay connection can work, but
does not isolate URL count/timing as the Cloudflare failure cause. The new
single-endpoint selection still needs a same-provider Cloudflare retest.

Production configuration now applies single-endpoint selection after resolving
credentials, including cache hits, for both Cloudflare and custom providers:
retain all configured STUN discovery URLs, select the first UDP TURN URL, or
prefer one TLS then TCP URL only when UDP is absent. The standard URL parser
recognizes implicit UDP (`turn:`) and implicit TLS (`turns:`). Credentials stay
with their original server entry. Saved settings and the credential cache keep
the complete server list. Force-off configuration retains every TURN URL and
its existing timed allocation tiers.

The selected endpoint is the only TURN URL passed to the peer, so later
transport tiers cannot delay gathering completion/readiness. Full host/srflx
signaling, relay-only connectivity guards and readiness semantics stay intact.
There is no automatic retry of alternative endpoints in forced mode. On a
UDP-blocking network the user can select Custom servers with a TLS/TCP-only
TURN URL; this preserves the forced routing policy without introducing another
connection lifecycle or silently falling back to direct traffic.

Regression tests cover mixed STUN/TURN entries, correct credential association,
implicit transports, stable first-UDP selection, TLS/TCP-only configurations,
missing TURN rejection, persisted complete settings and Force-off restoration.
The earlier isolated coturn relay-only/no-direct-fallback fixtures remain
applicable. The new packaged build still requires a real Parsec host retest.
