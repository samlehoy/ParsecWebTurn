# TURN renewal investigation

Investigate the user-reported ExpressTURN disconnect after roughly ten minutes
separately from the resolved Force TURN readiness problem. Cloudflare remains
stable in the user's comparison. No renewal behavior is changed in this probe.

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

The normal isolated TURN CI also runs these tests. No external service, account,
Parsec host or user settings are accessed.

## Live capture

Use the separate diagnostic executable with `account-network-audit`, the pinned
core, and `turn-diagnostics.json` as report path. `Start-TURN-Diagnostics.cmd` in
the test package supplies these arguments. Copy your existing `settings.json`
beside the diagnostic executable; settings and secrets are not packaged.

1. Start a fresh process and make just one ExpressTURN connection. Keep the same
   transport and Force TURN setting as the failing trial.
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
