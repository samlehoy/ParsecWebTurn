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
sync marker is fabricated, and its placeholder address is never used as a peer.
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
