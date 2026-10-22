# Local video timing and display policy

The native video worker collects constant-size latest/average/peak CPU durations
for both transports. No live pixel readback, GPU timestamp query, extra decoded
surface copy, per-frame logging, timer thread or dependency is added.

- **Compressed queue wait:** admission to the native compressed queue through
  removal by its decoder worker. Does not include BUD receive-ring/SCTP assembly
  or earlier network/guest dispatch time. Parameter-only messages also count.
- **Decoder API service:** a feed call's wall time minus time servicing decoded
  output callbacks. Includes native input preparation and driver/API stalls.
- **Decoder input to output:** bounded correlation of native input sequence IDs
  (vendor paths) or preserved MF sample timestamps. Missing, ambiguous or evicted
  IDs remain unknown. This includes native buffering and is not GPU execution.
- **Decoded surface handoff:** CPU observation of available output until its
  presentation handler; there is no added decoded-frame queue.
- **Presentation preparation:** CPU submission of the existing conversion,
  scaling and overlay work, or original-surface registration/configuration.
- **Present API call:** CPU wall time inside DXGI/Presentation API Present.
- **Present submission interval:** time between accepted DXGI Present calls,
  including idle periods. This is submission cadence, not measured scanout.

Decoder service is per compressed message; output metrics are per observed
output. These averages have different denominators and must not be summed as
end-to-end latency. Input-to-output overlaps other metrics. Windows/driver
buffering, compositor work and actual scanout are not measured.

## Display timing (experimental)

Video settings offers Automatic (existing behavior), Low latency and VSync.
Reconnect to apply. Automatic keeps the original flip-discard swap chain and
Present(0, DO_NOT_WAIT). The explicit modes request a documented waitable swap
chain and maximum frame latency of one. The worker polls readiness with a zero
timeout before conversion. A consumed slot
is retained until a presentation is actually accepted. Resize retains flags.
When the newest RGB output meets a busy display queue, its existing backbuffer
is retained and only Present is retried. In Low latency mode, ready pending
output is serviced even while more compressed input is available, after stop
and generation-reset checks. When input is empty, the worker waits on both
the documented DXGI readiness handle and one queue event; new input, recovery
or stop interrupts this wait. The queue event is reset under the queue lock
to avoid lost wakes, and a consumed DXGI signal is cached until Present.
No queued input waits for monitor readiness. Automatic and VSync retain their
previous wake policy. A short retry timeout remains for unsupported event
control or a busy Present after readiness. No decoded surface, extra RGB
target, repeat conversion or new timer thread is needed.
VSync uses Present(1, DO_NOT_WAIT); Low latency uses interval zero. No explicit
tearing flag is enabled, and neither mode guarantees physical scanout behavior.
Unsupported queue control falls back without failing decode, with a warning and
an effective-mode indication. Device loss still fails normally.

Original-surface Zero copy keeps its Windows Presentation API timing and bounded
surface leases. It is instrumented but not silently replaced with RGB rendering
to implement this selector. Its decoded-output correlation is reported only for
outputs that reach presentation.

References: [DXGI latency control](https://learn.microsoft.com/en-us/windows/uwp/gaming/reduce-latency-with-dxgi-1-3-swap-chains),
[Present intervals](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgiswapchain-present),
[Presentation target time](https://learn.microsoft.com/en-us/windows/win32/api/presentation/nf-presentation-ipresentationmanager-settargettime).

## BUD burst catch-up

The existing transport can skip to a complete keyframe. This change separately
addresses the downstream native decoder queue: when at least two later messages
are queued, BUD decodes every reference but omits obsolete display updates.
Normal rendering avoids their conversion/output write; original-surface rendering
avoids submitting their leases. The last two queued messages keep normal display
eligibility. WebRTC display policy and all transport timers/reliability are unchanged.

The deterministic 24-picture test uses 2 ms decode and 4 ms display submission
cost: ordinary processing takes 144 ms/24 display submissions; catch-up takes
56 ms/2 submissions, with identical decoding of all 24 references. With zero
display cost, or a single message, there is no modeled time improvement. This is
a scheduling model, not evidence of measured live latency or codec performance.
The isolated `video-hardware-probe report.json --timing` additionally exercises
all DXGI modes and resizing with synthetic NV12 surfaces, without account access.
Unsupported GPU cases are explicitly reported, not treated as successful output.

## CPU and memory scope

Eight fixed-size duration accumulators and a 64-slot input-identity table are
used instead of storing per-frame histories. The identity table retains metadata only, no picture bytes
or COM surfaces. Existing 32-message/8 MiB compressed queue and 8 MiB reuse-pool
budgets remain; no larger picture buffers or extra worker is introduced.
The worker marks its queue stopped before clearing it, so a retired pipeline's
compressed-buffer reuse pool is released even if its owner still retains stats.

Connection stats queries process memory only on its existing visible-window
timer. Optional event logging records a few startup milestones (core load,
guest start, initial graphics/callbacks), not periodic or per-frame memory events.
It distinguishes total resident working set, private commit, and private working
set where supported by Windows. These are different quantities; private commit
can exceed RAM currently resident. The immutable WASM imports 174 initial 64 KiB
pages (10.875 MiB), with growth allowed by its existing contract. This is not a
measurement of the whole process. Driver caches and compiled module pages need
separate observation. No working-set trimming or forced garbage collection is
used to manufacture a lower Task Manager number.

An intermittent 200–250 MB versus 38–40 MB has been reported, with later
observation of memory decreasing while the process remains open. Its cause and
a lower steady-state target are not established; compare resident and committed
values over time before changing allocation policy.

The Low latency scheduling candidate addresses idle retry quantization and
pending-output starvation. Win32 event tests verify input interruption,
simultaneous-signal priority and one-time readiness consumption without GPU
readback. These tests do not prove smooth physical presentation; compare live
Automatic/Low latency on the same stream and monitor before claiming a gain.
