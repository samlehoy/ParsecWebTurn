# Synthetic H.264 decode fixture

## HEVC Main 10 diagnostic fixture

`synthetic-720p-hevc10.bin` is the synthetic, length-prefixed 120-picture
1280x720 Main 10 4:2:0 fixture from MIT-licensed nomi-san/lowlat,
revision `5f8db5689e55de0d396b0520824caa9fc053dd86`,
`crates/decode/tests/data/synthetic-720p-hevc10.bin`.
SHA-256: `80144564475de3bc786d18a45e9c3395c5841b72a2d4c040da513031a936fc1c`.
It contains no captured desktop or account data. It is embedded only in the
separate diagnostics build, not the normal client. Upstream MIT attribution is
retained in the lowlat license notices shipped with the project.

Run the diagnostics binary with `video-hardware-probe report.json --main10`
to exercise the actual normal GPU P010 decode/conversion path without a host,
account, network, pixel readback or vendor decoder. Capability unavailability is
reported explicitly. This is not proof of stock-host negotiation, display color
correctness or reduced latency; the MFT may retain its final picture until a
next input because this probe does not inject an end-of-stream drain into the
live pipeline.

Local validation of client source `cf24d12` on AMD Radeon 780M is recorded in
[`main10-gpu-2026-10-16.json`](../../docs/measurements/main10-gpu-2026-10-16.json):
120 inputs, 119 decoded/presented pictures, P010 D3D11 decoder surfaces,
zero dropped pictures and released resources. The final picture was retained
by the streaming MFT. This establishes the GPU-surface path; it does not measure
hardware execution counters or establish stock-host Main 10 negotiation.

## H.264 fixture

Generated locally from FFmpeg's synthetic testsrc2 filter; no captured media,
account data, credentials or external video source. Eight 1920x1080 High-profile pictures: one IDR followed by seven delta pictures.
The fixture is test input, not third-party program code. FFmpeg is not shipped.

Generation:
```
ffmpeg -f lavfi -i testsrc2=size=1920x1080:rate=60 -frames:v 8 -c:v libx264 -pix_fmt yuv420p -profile:v high -preset ultrafast -tune zerolatency -x264-params "keyint=30:bframes=0:repeat-headers=1:aud=1:cabac=1:8x8dct=1" -f h264 synthetic-1920x1080.h264
```

The native hardware probe splits on access unit delimiters, runs the actual
Media Foundation/D3D11 path, and optionally samples the synthetic GPU backbuffer
once to verify pixel variation. Readback is disabled in live sessions.
