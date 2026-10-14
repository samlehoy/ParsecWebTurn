# Isolated second-stage latency screening

These are standalone research executables, not modules of ParsecWebTurn. No app
settings, account, host, public server or real media is used. None is packaged
with the normal client. Run the native validation workflow manually on dev with
`latency_probes=true` to build them and collect Windows/Linux UDP reports.

## UDP burst screening

`udp-burst` compares the actual Tokio `recv_from().await` operation used by the
underlying transport with a bounded `readable().await` / `try_recv_from()` drain
of at most 32 packets followed by a scheduler yield. This is an experiment, not
a selected production implementation. Both modes use the same requested 1 MiB
socket buffer; the granted value is recorded. A separate sender thread emits
256 bursts of 16, 64 or 256 datagrams of 1200 bytes, with 2 ms between bursts.
The first 16 bursts are warmup. Five paired rounds alternate order. Reports
include missing/duplicate datagrams, receive latency, the first packet of each
burst as a control proxy, and lateness of a competing 1 ms Tokio timer.

This measures application enqueue-to-UDP-receive time on loopback. It includes
the sender syscall, OS scheduling and queueing; it is not NIC arrival time. It
does **not** include ICE, TURN, DTLS or SCTP, and cannot justify changing the
WebRTC client by itself. A repeatable benefit without increased loss or worse
competing-task latency is only grounds for a subsequent full WebRTC experiment.
`readable` draining also performs one receive syscall per datagram: it is not
Linux `recvmmsg` or Windows registered IO. A large benefit is not assumed, since
Tokio's existing async receive already handles ready data without a new sleep.

## Decoder availability

Run `decoder-wait.exe > decoder-wait.json` on a Windows PC with working D3D11
video support. Hosted CI only builds this executable; it cannot establish
hardware performance. The program creates no window and performs no GPU pixel
readback or presentation. It feeds the existing synthetic eight-picture H.264
fixture through the documented Media Foundation/D3D11 path at 60 FPS, repeating
it ten times per trial. The first 16 pictures are warmup. Five paired rounds
alternate the documented H.264 low-latency property off/on, recording whether
the request was accepted.

Each output is matched to its input using the sample timestamp. Measurements
start just before `ProcessInput` and end when the corresponding GPU output
sample is returned by `ProcessOutput`. They record output-availability latency,
how many later pictures have been submitted, and pending pictures before a
final drain. The final drain is only for measurement completeness, never a
proposed live-session operation. All 80 inputs must produce matched outputs.
Decoder-specific texture binding is evidence of a DXVA surface, not an engine
execution guarantee. The measured time is not GPU completion or display latency.

The fixture has no B pictures and is unchanged between modes. This does not
compare rewritten SPS/VUI or HEVC; it establishes whether our already requested
H.264 mode avoids a detectable extra decoder hold on this PC. A negative result
means no additional client change has been justified. A result from one GPU
cannot establish the behavior of another driver or the user's notebook.
