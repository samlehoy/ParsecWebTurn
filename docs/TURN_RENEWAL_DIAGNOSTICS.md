# TURN renewal investigation

Investigate the user-reported ExpressTURN disconnect after roughly ten minutes
separately from the resolved Force TURN readiness problem. Cloudflare remains
stable in the user's comparison. Baseline renewal behavior remains unchanged.
The separate opt-in timing experiment below changes only ChannelBind refresh.

## Why capture renewal

[RFC 8656 sections 3.2, 8 and 12](https://datatracker.ietf.org/doc/html/rfc8656)
define allocation refresh and channel-binding refresh. The default allocation
lifetime and the channel-binding lifetime are ten minutes. Sending media does
not itself refresh the allocation. The server supplies the actual allocation
lifetime; a stale authentication nonce requires retry with its replacement.

ExpressTURN's [public service page](https://www.expressturn.com/) does not
document a ten-minute session limit. This does not establish the limits of a
particular account. A provider difference alone does not prove either a server
fault or a client fault.

The pinned `turn` 0.11.0 client starts allocation refresh at half the initial
lifetime and permission refresh at 120 seconds. ChannelBind refresh is initiated
after five minutes when sending data. Source inspection identifies these risks:

- Allocation Refresh error responses other than 438 can return `Ok(())`.
- The allocation/permission retry loops continue even after success.
- ChannelBind does not retry a 438 response using the supplied nonce.
- The periodic allocation timer retains the initial interval even if a later
  successful Refresh response reports a shorter lifetime.

These are library observations, not proof of the ExpressTURN incident's cause.
Do not change service policy, nonce handling or timers before observing which
path the live failure uses.

## Isolated observations

`tests/turn-integration/src/renewal.rs` uses a loopback UDP endpoint and synthetic
credentials. It exercises the actual pinned public TURN client with a short
allocation lifetime, a 438 challenge and a rejected Refresh. A separate scenario
returns ChannelBind 438 and checks whether another authenticated bind is sent.
The current-behavior observation is explicitly labeled; update it when fixing
the library rather than treating those limitations as the desired contract.

Verified in [CI 38399910659](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/38399910659):
25 isolated tests passed, including both new renewal observations. The client
retries allocation 438 with the rotated nonce. A rejected Refresh produces
three requests; ChannelBind 438 is not retried even after a subsequent send.

The normal isolated TURN CI also runs these tests. No external service, account,
Parsec host or user settings are accessed.

## Live capture

Use the separate diagnostic executable with `account-network-audit`, the pinned
core, and `turn-diagnostics.json` as report path. `Start-TURN-Diagnostics.cmd` in
the test package supplies these arguments. Copy your existing `settings.json`
beside the diagnostic executable; settings and secrets are not packaged.

1. Start a fresh process with just one ExpressTURN URL and make one connection.
   Keep the same transport and Force TURN setting as the failing trial.
2. Keep the stream active for at least 15 minutes or until error 6200 occurs.
3. After the error, close the client normally so it writes the report. Do not
   reconnect in that process. Forced termination may prevent report export.
4. Send `turn-diagnostics.json` and the approximate disconnect time.
5. Optionally repeat in a fresh process with Cloudflare for the same duration.

`native_transport_diagnostics.turn_renewal_events` contains fixed event labels,
numeric error codes/lifetimes, and milliseconds from the first observed renewal
event in that process. It retains the last 128 observations and counts omitted
events. No raw server reason, username, password, nonce, peer IP or packet payload
is retained by this added trace. Other existing report sections retain their
existing destination/candidate information; review before sharing publicly.

The timeline is process-scoped, not proof that a particular allocation belongs
to the selected route. Keep one attempt per process to reduce ambiguity. A
Refresh with requested lifetime zero during shutdown is intentional deletion.
Missing events remain unknown, particularly if diagnostics were not enabled or
the process did not export a report. Library log observations are supplementary
diagnostics; they never control routing or connection state.

Look for the initial lifetime, Refresh attempts/responses around half that
interval, 438 and the subsequent success or failure, channel-binding refresh
and error codes, and the final ICE/DTLS/SCTP state. Use that sequence to choose
the next fix; the approximate ten-minute interval alone is insufficient.

## Controlled ChannelBind timing experiment

Two user reports show failure at 302.858 and 302.233 seconds from attempt start
(a difference of 0.625 seconds). Their renewal timelines both place teardown
immediately around the first ChannelBind refresh at approximately 300 seconds.
The second trial kept the machine unlocked. All recorded allocation, permission
and channel-binding refresh responses were successful; no 438 was recorded.
The most recent allocation response reported another 240 seconds of lifetime
at approximately 240 seconds. The client's later lifetime-zero request is
cleanup after termination, not evidence of a server-expired allocation.

This timing correlation does not prove that ChannelBind is the cause, or whether
any fault belongs to the client, the provider, or the surrounding network.

Only a build with the `diagnostics` feature can opt in using
`PARSECWEBTURN_TURN_EARLY_BIND_REFRESH=1`. The fixed diagnostic value is 60 seconds;
missing or different values retain 300 seconds. Normal builds omit this feature
and ignore the variable completely. No connection policy, readiness, allocation
or permission timer, retry, authentication, SCTP, media or input behavior changes.

The setting is read once per relay connection. The existing on-send trigger is
retained: refresh runs on the first send after the threshold, not on a new exact
periodic timer. The trace reports `channel-bind-refresh-interval-seconds`, so
verify a value of 60 in the experimental run. A separate loopback test, executed
in its own process, waits beyond 60 seconds and verifies a refresh of the same
channel number before the allocation or permission timer can fire.

Run the 60-second test launcher using the same ExpressTURN URL, credentials,
transport and Force TURN setting as before. Keep the machine unlocked and the
stream active until failure, or for at least 15 minutes. After an error close
normally and retain the report; do not reconnect in that process.

If failure follows the first refresh to approximately 60 seconds, the refresh
path becomes a strong causal lead. If early refreshes succeed and failure stays
near 300 seconds, investigate another five-minute mechanism. If it stays stable,
repeat or compare the default 300-second mode before calling this a fix. Keep
all of these outcomes separate from the already working Force TURN readiness.
