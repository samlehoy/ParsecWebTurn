# Native TURN routing and core hotkeys

## Routing

`forceRelay` defaults to false, including for existing settings files. Enabled,
it applies the standard `RTCIceTransportPolicy::Relay` to the next peer. There is
no automatic downgrade to direct ICE. The UI clears this choice when STUN-only
is selected; contradictory JSON and custom configurations without TURN servers
are rejected. Credentials and policy use one immutable settings snapshot.

The native client opts into a small vendored ICE gathering extension; its default
remains disabled for other users. UDP TURN starts immediately, TLS after a
three-second head start for UDP, and plain TCP after another three seconds.
Missing tiers add no delay. Delayed fallbacks stop when ICE has already connected
or closed. The policy considers connectivity, not just successful allocation:
an allocated UDP relay that cannot reach the peer must not prevent fallback.
The wait is a bounded preference window, **not proof that UDP is impossible**.
Very slow UDP establishment can therefore allow TLS to win.

Relay candidates additionally use RFC 8445 local preference to rank UDP above
TLS above TCP, retaining relay type preference zero and component one. Direct
candidate priorities, nomination roles, STUN checks and certificate validation
are unchanged. The remote controlling agent still owns nomination. TURN over
TCP/TLS remains a UDP allocation toward the peer, not ICE-TCP. An established
session is not periodically restarted to try another transport.

References: [W3C relay policy](https://www.w3.org/TR/webrtc/#dom-rtcicetransportpolicy),
[RFC 8445 candidate priority](https://www.rfc-editor.org/rfc/rfc8445.html#section-5.1.2.1).
The loopback coturn fixture verifies actual three-channel DTLS/SCTP traffic and
selected-relay telemetry with all transports available, UDP unavailable, and
UDP plus TLS unavailable. It uses trickle ICE so an unanswered allocation does
not hold up a working fallback until gathering completes.

## Original 150-104a hotkeys

The original core owns `hotkey.json`, its settings UI, recording, conflict checks,
action IDs and execution. The bounded virtual profile already persists this file.
No guest code or configuration is patched. `mty_app_set_keys` registers original
Matoya codes; native key events enter the exported `mty_window_keyboard`.
It translates browser modifier bits to Matoya flags, then calls
`mty_app_kb_to_hotkey`: matching keydown becomes event type 6 with the configured
action ID, while matching keyup is consumed (type 0). Other keys remain type 5.

Native F8/F11, Ctrl+comma and Ctrl+Shift+S previously intercepted keys before
that export. `core_hotkeys.rs` now reads the pinned app's hash at +32 only when
arbitrating a native shortcut. `MTY_HashSetInt/GetInt` use `#%llx` strings for
`((MatoyaModifiers & 0xff) << 16) | MatoyaKey`. Bounds, action IDs, bucket/header
stability and address arithmetic are checked. Changes are read immediately;
there is no separate Rust hotkey configuration cache. An empty hash (used during
recording) or unavailable/changing data yields the key to the core.

This is a reverse-engineered, read-only contract for the SHA-pinned core, not an
SDK API. Updating the core requires another audit. An isolated test uses the
unchanged original keyboard export with a synthetic app/hash and callback to
verify custom F11 dispatch, modifier matching, release consumption, ordinary
input and the reader's recording/error behavior. It never loads a real profile.

The web core ignores the per-hotkey global flag and its
`MTY_AppEnableGlobalHotkeys` is empty. These bindings work with client focus;
this change does not register system-wide shortcuts or promise background
activation. Secure OS combinations remain controlled by Windows.

## Automated validation

Runtime changes at `2ac9dda` passed normal and diagnostics Windows unit tests,
Clippy in both configurations, isolated Settings save/reopen and immersive
capture tests, and the loopback coturn fallback/SCTP tests on 2026-10-12.
[CI run](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/38268921429).
The generated Settings preview also confirms the new checkbox remains reachable
without overlapping the footer. Actual Parsec-host routing and UI hotkey
rebinding still need the user's live test; the synthetic fixtures do not prove
interoperability with every host or network.
