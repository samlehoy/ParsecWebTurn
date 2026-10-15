# Intel HEVC Media Engine investigation

The affected Intel(R) Graphics laptop (Core Ultra 5 235U, driver
32.0.101.8860) ran the standalone `0d8a59f` HEVC Main 8-bit probe.

| Path | Observed result |
| --- | --- |
| Existing synchronous MFT | `E_FAIL (0x80004005)` |
| Direct D3D11 | No supported decoder profile for the stream |
| Media Engine / DirectComposition | Playback, 1280 x 720, no reported drops |
| Same Media Engine path, real-time mode | Playback, same geometry, no reported drops |

Both Media Engine runs had valid process-and-selected-adapter VideoDecode
counter samples (40 and 39), but zero positive samples. Hardware decoding
remains **unproven**. Software decoding followed by GPU presentation is a
possible explanation, not an established verdict. Neither playback nor GPU
output meets the hardware-only integration requirement on its own.

The next diagnostic uses Microsoft's documented
[MFTrace](https://learn.microsoft.com/en-us/windows/win32/medfound/using-mftrace)
to inspect instantiated components, resolved topology and actual transform
input/output, including D3D/DXVA negotiation and fallback where exposed.
A decoder CLSID/module name or successful D3D-manager handoff alone must not
be classified as hardware execution. If this trace does not establish it,
GPU ETW analysis will still be needed; no heuristic replaces that evidence.

`Run-Intel-HEVC-Trace.cmd` only launches the existing synthetic offline probe,
with no live-client/WASM change. It starts the probe under the ordinary account,
waiting before any codec discovery. A generated helper attaches elevated MFTrace
to that exact PID, so using a different administrator does not change the probe's
account. Its CMD helper fetches only the original Microsoft
SDK payload and extracts two hash-pinned tools without system installation.
See [diagnostic instructions](INTEL-HEVC-TRACE.txt).

Tool acquisition was checked using the Windows SDK 10.0.22621 payload recorded
in Mozilla's [SDK package manifest](https://github.com/mozilla/gecko-dev/blob/master/build/vs/vs2022.yaml):
`69661e20556b3ca9456b946c2c881ddd.cab`, SHA-256
`ee19778f3b0d943d1d18a33d03ec26f2efe0d4e3bb5ce91c2d10e810f4392f80`.
The extracted x64 binaries have valid Microsoft Corporation signatures.
The helper does not redistribute these tools in the test ZIP.

Tool acquisition, hash checks and Microsoft signatures were validated locally.
The complete MFTrace NVIDIA control run was not obtained:
the agent session is not elevated and MFTrace requested elevation before
launching the probe. This limitation must not be reported as a successful
hardware-control trace.

## First laptop trace: different account, no HEVC decoder

The user supplied an MFTrace capture from the same probe binary (SHA-256
`108215a6f7aad4929db47671c6e42b3056dd23890e204c9bb61a553b87bc5ecb`).
The user confirmed this run launched the probe under a different administrator
account. Unlike the original normal-account run, the synchronous path found
no HEVC transform and both Media Engine trials failed during topology resolution
with `MF_E_TOPO_CODEC_NOT_FOUND (0xc00d5212)`. The input HEVC stream was read,
but no HEVC decoder activation/input/output was captured. `msvproc.dll` was
instantiated as a video processor; this is not an HEVC decoder or hardware proof.

These results do not identify the decoder used in the previously successful
playback. A changed account/codec-registration context is the leading explanation,
not yet a demonstrated root cause. The revised attach flow preserves the ordinary
account and instruments it before MFStartup. A new laptop capture is pending.

## Second laptop trace: working decoder identified, hardware still unknown

The ordinary-account attachment run found HEVC Video Extensions 2.5.33.0,
`HEVCDECODER_STORE.dll`. Both Media Engine trials successfully configured the
same package's HEVC-to-NV12 transform, fed compressed inputs and received
decoded outputs. The first captured transform had 538 input / 529 output log
entries; the real-time trial had 543 / 543. These are trace-entry counts across
looped playback, not distinct-frame throughput or latency measurements.
The JSON reported playback without drops in both trials, with 40 and 39 valid
VideoDecode counter samples but no positive samples. The synchronous baseline
again failed with E_FAIL; direct D3D11 still found no supported HEVC profile.

The trace confirms which decoder actually processed the stream. It does not
expose enough of that decoder's internal acceleration decision to establish
hardware execution or software fallback. D3D-awareness and displayable-resource
attributes are capabilities/configuration, not execution evidence.

The next optional offline diagnostic combines the built-in Windows WPR GPU
profile with a small event-provider profile based on Microsoft's public MF
tracing profile. This includes HEVC decoder events and hardware-scheduled GPU
queue events, preserves the ordinary-account target and gives each collection
a unique instance. Trial UTC timestamps and target PID allow event correlation.
See [GPU trace instructions](INTEL-HEVC-GPU-TRACE.txt). No software decoder or
Media Engine backend is integrated into the live client on this evidence.
