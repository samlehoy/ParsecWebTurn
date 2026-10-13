# Force TURN routing and fallback

## Current policy

Force-off is unchanged: resolve the complete configured ICE server list, allow
normal ICE route selection, and retain the existing UDP/TLS/TCP allocation tiers.
The sequential retry plan exists only for a user connection started with Force
TURN enabled. It snapshots the resolved credentials once; later endpoints in
that same chain reuse them without another credential HTTP request. The plan is
released after success/cancellation, not persisted to disk or shared with a new
user connection. Saved settings and the provider credential cache keep all URLs.

Force-on retains host and STUN discovery/signaling while restricting actual ICE
connectivity and DTLS/SCTP data to local relay candidates. The project-owned
`SettingEngine::set_relay_only_connectivity(true)` defaults off. This is a
routing restriction, not the address privacy of W3C `iceTransportPolicy: relay`:
Parsec still receives the real host/srflx candidates. The literal peer policy is
All so this discovery remains possible. Relay guards cover pair creation,
selection, inbound checks, outbound checks and non-STUN data validation.

## One endpoint per attempt

Expand TURN server entries into a stable endpoint list, preserving each URL's
own credentials. Keep all STUN URLs in every attempt. Deduplicate exact
URL/credential duplicates. Prefer UDP, TLS, then plain TCP, maintaining provider
order within each tier. Implicit UDP (`turn:`) and TLS (`turns:`) use the pinned
standard URL parser. Each peer receives exactly one TURN URL.

The original compact Parsec candidate mapping stays intact: host has `lan=true`,
srflx has `fromStun=true`, relay has both false. Do not invent addresses or alter
flags to make a relay look like another candidate type. The ready marker stays
one-shot and follows actual gathering completion plus remote description and
at least one gathered relay. The Force-only candidate gate accepts validated
remote candidates once the remote description is ready. Ordinary marker timing
and candidate buffering remain unchanged.

## Failure and bounded retry

Only pre-establishment route failures (ICE/peer transport failure or the short
relay endpoint timeout) advance the plan. Host rejection, malformed signaling,
DTLS verification failure, user cancellation and failures of an established
stream do not advance it. After a host answer, an endpoint with no DTLS progress
gets 8 seconds, allowing several connectivity checks. This is an application
policy, not an IETF guaranteed minimum. DTLS Connecting/Connected permanently
disables that short timer; completion retains the ordinary 30-second deadline.
The last endpoint also retains the ordinary deadline. All attempts share a
60-second chain budget starting after credential resolution. The normal client
keeps its original establishment timeout.

Before a retry, cancel the old attempt, revoke guest output leases and discard
its pending candidates/control/media. Wait for confirmed worker completion and
peer closure before dispatching another connection. Cleanup/event handoff has a
3-second bound; failed cleanup stops the chain. Pending retries are cancelled by
new user key/button commands, Force being disabled, or app shutdown. No direct
connection is used as a fallback.

## Original core reconnect binding

The immutable SHA-pinned 150-104a core owns connection signaling. Do not reuse an
old offer, ICE credentials or attempt ID for the next TURN endpoint. The native
GUI worker dispatches the core's own connect-current-peer UI action, so the core
creates the fresh attempt and performs the signaling exchange itself.

Audited read-only WAT evidence for this specific core:

- Function table 84 is `main_render_loop`; its opaque pointer is `app_args`.
- Function table 80 is `main_app_ui_event(app_args, event, payload)`.
- Event 7 ignores payload and queues message type 6 for the selected client.
- `main_render_loop` calls `main_submit_ui_events`, which consumes message 6
  and calls `app_native_run_client`. Event 6 instead queues message 4 and
  must not be used for this reconnect operation.
- That function reads the current configured peer, generates a new random
  attempt ID, calls `parsec_web_new_attempt`, and builds the signaling offer
  with the newly returned ICE credentials.

These are private Parsec ABI facts for the pinned binary, not a public SDK or a
W3C reconnect API. Keep the binding isolated to this GUI worker, validate the
context/signature, and re-audit before any core replacement. The core itself
is never modified. An intermediate cancelled/idle status (-3), not a false
success, retires the old connection; final failure still uses the normal -6200
report. Reconnect is dispatched only after the cancellation event is consumed.

## Evidence and limits

The user verified that a single-endpoint forced connection succeeds with both
ExpressTURN and Cloudflare. Earlier multi-endpoint Cloudflare attempts gathered
relay candidates and exchanged peer sync, but every relay pair failed. Ordinary
connections worked. These observations do not establish whether multiple relay
candidates, delayed readiness, or another timing interaction caused that failure.
Sequential fallback retains the verified one-endpoint path rather than changing
simultaneous multi-endpoint signaling.

Regression coverage includes endpoint ordering, discovery/credential retention,
deduplication, unchanged Force-off configuration, bounded plans/timers, progress
and stream protection, cancellation guards, and callback signatures in the
actual pinned module. Existing isolated coturn tests cover UDP/TCP/TLS, compact
candidate exchange, full discovery with relay-only connectivity, payload
transfer and rejection of a usable direct path. Automated checks do not prove
real Parsec host retry behavior; the automatic second-endpoint path needs a
live test with the preferred endpoint deliberately unavailable.

Separate diagnostic-build limitation retained from the earlier investigation:
TURN send errors acquire an `io error: ` prefix, while the permission error
subclassifier matches an unprefixed CreatePermission string. Zero 403/other
subcategory counts therefore do not prove permission errors were absent. The
aggregate send-failure count remains meaningful. This diagnostic output is
never a production routing input and is excluded from the normal executable.

References: [Cloudflare TURN transports](https://developers.cloudflare.com/realtime/turn/),
[RFC 8838](https://www.rfc-editor.org/rfc/rfc8838.html), and
[research references](NATIVE_REFERENCES.md).
