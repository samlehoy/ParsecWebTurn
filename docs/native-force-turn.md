# Force TURN routing and readiness

## Current policy

Force-off is unchanged: use the complete ICE server list, normal route selection,
and existing timed UDP/TLS/TCP allocation tiers.

Force-on also retains the complete resolved server list, including each URL's
credentials and STUN discovery. There is no single-endpoint filter, sequential
retry plan, shortened endpoint timeout, automatic core reconnect action or shared
retry budget. The ordinary connection establishment deadline remains in place.
Saved settings and the credential cache retain their normal behavior.

Force-on retains host and STUN discovery/signaling while restricting actual ICE
connectivity and DTLS/SCTP data to local relay candidates. The project-owned
`SettingEngine::set_relay_only_connectivity(true)` defaults off. This is a routing
restriction, not the address privacy of W3C `iceTransportPolicy: relay`: Parsec
still receives real host/srflx candidates. The literal peer policy is All so this
discovery remains possible. Existing relay guards cover pair creation, selection,
inbound/outbound checks and non-STUN data validation.

## Earlier application readiness

The Parsec sync marker is an application-level readiness message, not an ICE
candidate or end-of-candidates indication. Force-on queues it once both the remote
description is successfully set and at least one real local relay candidate has
been queued for signaling. The candidate event precedes the marker in the same
bounded queue. Either callback order works: relay first or remote description first.

Do not wait for `on_ice_candidate(None)`. The later TLS/TCP allocations can finish
and their candidates can still be signaled after the marker, using the existing
trickle path. Host/srflx discovery stays enabled. The marker remains one-shot;
cancelled/closing attempts cannot emit it, and queue overflow fails the attempt.
Ordinary connection marker timing and remote candidate buffering are unchanged.

## Evidence and limits

The user verified successful forced connections with one ExpressTURN URL and
later with one Cloudflare URL. Multi-URL forced attempts failed, while ordinary
routing could select Cloudflare relay successfully. A sequential retry approach
was tried and has now been removed at the user's request.

The earlier failed Force-on implementation waited for all gathering before
sending readiness. TLS and TCP tiers started later, delaying that marker. This
makes early readiness a plausible fix, but endpoint count and timing changed
together in prior trials. The Parsec host's handling of later relay candidates
is private protocol behavior and still needs a live multi-URL host retest.
Do not describe this change as proven to fix Cloudflare before that retest.

Regression tests cover readiness without gathering completion, both prerequisite
orders, one-shot behavior, event order, cancellation and queue overflow. Settings
tests retain every configured endpoint and its credentials for Force-on and
Force-off. Isolated coturn fixtures cover relay connectivity and no direct fallback.
These fixtures do not replace a real Parsec host compatibility test.

The pinned WASM core is unchanged. The former sequential core UI reconnect binding
and its associated tests have been removed. Audio/video decoding, presentation,
and normal transport selection are unchanged.

## Known optional diagnostic limitation

The optional diagnostics wrapper records aggregate CreatePermission transactions
and errors, but the existing 403 subtype counter can miss the wrapped public error
type. A zero 403 subtype count is therefore not proof that no 403 response occurred.
Diagnostics remain optional and are not part of the normal package.

Reference: [lowlat signaling readiness](https://github.com/nomi-san/lowlat/blob/main/docs/04-signaling.md).
