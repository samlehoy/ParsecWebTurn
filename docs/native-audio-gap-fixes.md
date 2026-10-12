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
Short gaps after startup do not restart the core-requested initial buffering
threshold. Windows supplies silence when the endpoint runs out of data; the
next actual packet can resume at the next endpoint event. Initial buffering,
reset, supported engine-period discovery and the format-conversion fallback
are preserved. On a bounded playback-queue overflow the oldest queued PCM is
discarded on stereo-frame boundaries, keeping recent audio instead of dropping
all new packets behind an old backlog. Such recovery can still be audible.

References:
- [Microsoft: Rendering a Stream](https://learn.microsoft.com/en-us/windows/win32/coreaudio/rendering-a-stream)
- [Microsoft: IAudioRenderClient::GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiorenderclient-getbuffer)

Tests cover partial/empty output planning, overflow ordering, stereo boundaries
and reset. Existing guest-worker cleanup and silent WASAPI diagnostics remain
in validation. These checks do not establish audible quality on a live remote
connection: host audio, network bursts, endpoint scheduling and drift still need
live testing. No codec, resampler, video or network protocol was changed.
