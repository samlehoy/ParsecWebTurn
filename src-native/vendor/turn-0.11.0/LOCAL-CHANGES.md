# Local TURN changes

Base: crates.io `turn` 0.11.0. Original MIT/Apache-2.0 licensing is retained.
Changes are limited to `src/client/mod.rs`, `src/client/transaction.rs` and
`src/client/relay_conn.rs`.

## Reliable transport transaction timers

The original client assumes a UDP transport. TCP and TLS need RFC 8489 section
6.2.2 behavior: TCP provides reliability, so STUN requests must not be
retransmitted by the protocol layer. A response is awaited once, up to Ti.

The opt-in `Client::set_reliable_transport_timeout(Duration)` is called before
listening or sending requests. Native ICE selects the recommended 39.5-second Ti
for TCP/TLS. UDP retains the original timers. The timeout is configurable through
this explicit API; no huge RTO or overflow-dependent workaround is used.

An EOF/reset/read failure on the reliable connection fails pending transactions
and the relay's read channel immediately. Failed initial writes remove their
transaction. Timer cancellation follows transaction removal and client shutdown.
Credentials, message integrity, allocation/authentication and UDP peer relaying
are otherwise unchanged.

Regression tests in `src-native/tests/turn_stream.rs` use an unresponsive local
server to verify no duplicate request is sent, one configurable timeout, and
immediate pending-request failure when the transport closes. The separate coturn
fixture proves UDP/TCP/TLS DTLS/SCTP interoperability with this patch enabled.

Reference: https://www.rfc-editor.org/rfc/rfc8489.html#section-6.2.2

## Channel-binding renewal

The send-triggered ChannelBind refresh threshold is 240 seconds instead of 300.
This leaves a margin before the five-minute permission boundary and avoided the
reported ExpressTURN disconnect in a separate-client trial lasting over 35
minutes. ChannelBind also renews the corresponding permission under RFC 8656.
The allocation and permission timers, authentication, retries and transaction
handling remain unchanged by this threshold adjustment. There is no runtime
switch or diagnostic timing override.

Reference: https://datatracker.ietf.org/doc/html/rfc8656#section-9
