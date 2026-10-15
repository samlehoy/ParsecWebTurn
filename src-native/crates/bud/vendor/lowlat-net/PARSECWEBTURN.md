# Native UDP accounting seam

MIT source from nomi-san/lowlat feat/win-seams,
revision 5f8db5689e55de0d396b0520824caa9fc053dd86, crates/net/src.
Original copyright and license are retained in LICENSE.

This copy keeps native socket IO, batching/offload, wakeups, interface discovery
and the BUD shell unchanged. Unused browser/SCTP and standalone NAT-probe modules
and their dependencies are omitted. The normal WebRTC client does not use it.

The only data-path addition is byte accounting: Batch increments a plain u64
only after a successful OS send, by the existing buffer length (or datagram
length on the fallback path). Failed sends and empty flushes do not count.
Shell counts each received UDP payload before parsing/authentication. Successful
offload and ordinary sends have identical byte-count semantics.

The counters include encrypted BUD records, acknowledgements, retransmissions,
STUN and TURN framing/control on this UDP socket. They exclude IP/UDP headers
and the separate account HTTPS/WSS connection. OS send acceptance is not proof
of remote delivery. No extra syscalls, payload copies, locks or GPU operations
are introduced. The client publishes the existing cumulative counters once per
protocol pass. Real loopback byte checks run with offload allowed and disabled;
failed sends, empty flushes, receive accounting and direct/coturn propagation
are tested separately.
