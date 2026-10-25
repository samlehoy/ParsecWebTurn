# Intel H.264 decoder screening

Measured 2026-10-14 on the user's Core Ultra 5 235U integrated Intel Graphics,
driver 32.0.101.8860, VPL API 2.16. Probe source: `dda2cd3`.

**Decision: retain the existing low-latency Media Foundation path.** VPL can
decode into original NV12 GPU surfaces, but this screening does not establish
a meaningful playback latency improvement or a better presentation/copy path.
No live decoder integration is justified by these results.

## Validated scope

Both reports identify the same adapter LUID. Each decoder completed five trials
of 80 H.264 pictures from the same synthetic 1920x1080 fixture, paced at 60 FPS.
The first 16 outputs of each trial are excluded from output-availability
statistics. p50 uses the upper middle and p95 uses nearest rank.

Only Media Foundation trials with low latency requested and accepted are the
production baseline. All five completed with zero frames held before drain,
zero later inputs needed before output and 80 D3D11 decoder-surface outputs.
The disabled-mode control's roughly 200 ms delay is not the current client.

All VPL trials completed with correlated output timestamps, native NV12 textures
on the caller's D3D11 device, zero application decoded-frame copies and no CPU
pixel readback. Nine nonfatal warnings were handled per trial. Internal runtime
copies remain unknown. No presentation or pixel correctness was tested.

## Output availability

Values below are the median of the five trial statistics, in milliseconds.

| Statistic | Media Foundation, low latency on | Intel VPL | Difference |
| --- | ---: | ---: | ---: |
| p50 | 0.321 | 0.278 | 0.043 |
| p95 | 0.451 | 0.398 | 0.053 |

Per-trial p50 values in microseconds:

- Media Foundation: 370, 304, 321, 324, 268.
- VPL: 333, 272, 338, 278, 273.

The ranges overlap; VPL is not uniformly better. The programs ran sequentially,
not as randomized paired trials, so matching round indices do not establish
paired wins or statistical significance. These tiny API output-availability
differences must not be reported as an end-to-end or GPU decode speedup.

VPL's median-of-trials GPU completion p50 is 3.190 ms, p95 3.374 ms. Media
Foundation's report does not measure GPU completion, so those values have no
comparable baseline. VPL waits for completion only in this offline probe;
adopting that wait in playback would be a different, unmeasured pipeline.

## Remaining limits

The vendor runtime has not been connected to the renderer. GPU lease lifetime,
completion ordering, actual presentation latency and image correctness would
need separate acceptance before any switch. The user's requirement remains:
no CPU video fallback and no additional decoded-frame GPU copy relative to the
existing path. Capability alone is insufficient.

HEVC failed VPL DecodeInit with status -3 and the direct D3D11 Main/NV12 query
returned E_INVALIDARG. The user reports a known driver issue affecting browser
HEVC too. This is not evidence that the physical GPU lacks HEVC capability,
and it is separate from the successful H.264 comparison.
