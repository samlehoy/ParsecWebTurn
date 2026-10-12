# Native audio polling and short gaps

The pinned core's `main_audio_loop` drains `parsec_web_poll_audio`, then calls
`MTY_Sleep(1)` (20 ms on its inactive/reset path). `MTY_Sleep` reaches the
existing WASI clock subscription through libc nanosleep. Adding a 60 Hz sleep
outside this worker callback delayed each new poll independently of WASAPI's
event-driven rendering. The host now retains only a 1 ms minimum iteration
guard for callbacks that return without waiting. The main display loop is
unchanged, and returning normally from the worker still executes the guest's
audio destruction and pthread-exit epilogue.

WASAPI shared mode accepts a packet smaller than the currently available buffer.
The output now submits only actual PCM. It no longer fills the rest of every
available buffer with silence and then counts that silence as queued playback.
Short gaps covered by endpoint padding do not restart buffering. Actual starvation
(both device padding and queued PCM empty) re-enters the core-requested minimum
threshold instead of repeatedly starting with a single short packet. On overflow,
old queued PCM is discarded on stereo-frame boundaries down to that minimum,
restoring headroom rather than repeatedly clipping at the maximum ceiling. The
configured minimum/maximum are unchanged. Recovery can still produce an audible
pause or discontinuity; samples missing during an upstream stall cannot be
recreated. Engine-period discovery, partial PCM writes, reset and the format
conversion fallback are preserved.

The user's 6d7bf9d live capture supplied the evidence for this recovery change:
zero Opus decode errors/dropped decoder packets and at most about 7.2 ms between
guest audio polls, but an output queue alternating between empty and its 150 ms
ceiling. The completed output reported 162,912 dropped PCM frames (about 3.39 s)
and 669 old-style empty output wakes. Some early empty wakes were during a source
pause, so these counters must not be equated to audible clicks. The capture does
not prove whether arrival bursts originated in the host, transport or network.

References:
- [Microsoft: Rendering a Stream](https://learn.microsoft.com/en-us/windows/win32/coreaudio/rendering-a-stream)
- [Microsoft: IAudioRenderClient::GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiorenderclient-getbuffer)

Tests cover partial/empty output planning, starvation versus padding-covered
gaps, refill hysteresis, burst recovery headroom, stereo ordering and reset.
Existing guest-worker cleanup and silent WASAPI diagnostics remain
in validation. These checks do not establish audible quality on a live remote
connection: host audio, network bursts, endpoint scheduling and drift still need
live testing. No codec, resampler, video or network protocol was changed.
