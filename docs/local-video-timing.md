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

Decoder service is per compressed message; output metrics are per observed
output. These averages have different denominators and must not be summed as
end-to-end latency. Input-to-output overlaps other metrics. Windows/driver
buffering, compositor work and actual scanout are not measured.

## Display timing (experimental)

Video settings offers Automatic (existing behavior), Low latency and VSync.
Reconnect to apply. Automatic keeps the original flip-discard swap chain and
Present(0, DO_NOT_WAIT). The explicit modes request a documented waitable swap
chain and maximum frame latency of one. The worker polls readiness with a zero
timeout before conversion; it never waits for display refresh. A consumed slot
is retained until a presentation is actually accepted. Resize retains flags.
When the newest RGB output meets a busy display queue, its existing backbuffer
is retained and only Present is retried while the input queue is idle. The
worker uses its existing condition variable with a short retry timeout only
while that output is pending; new input wakes it immediately. No decoded
surface, extra RGB target, repeat conversion or new timer thread is needed.
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

Seven fixed-size duration accumulators and a 64-slot input-identity table replace
unbounded histories. The identity table retains metadata only, no picture bytes
or COM surfaces. Existing 32-message/8 MiB compressed queue and 8 MiB reuse-pool
budgets remain; no larger picture buffers or extra worker is introduced.

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

An intermittent 200–250 MB startup versus 38–40 MB has been reported. Its cause
and a lower steady-state target are not established; compare startup logs from
both cases before changing allocation policy.
