# Offline Y410 to RGB10 diagnostic

Extract the whole artifact into a writable folder and run **Start-Y410-Probe.cmd**.
No administrator rights, installation, Parsec host, credentials or settings are
needed. Close the live stream first to avoid interfering with its GPU workload.
Share `y410-probe.json` and `y410-console.txt` afterwards.

This is an explicitly selected ignored Rust test, packaged separately from the
normal client. The launcher selects only `compare_y410_conversion_paths`, not
other UI/network/input tests. The normal application cannot invoke this probe.

For each DXGI hardware adapter it reports Y410/RGB10 format support, the explicit
format/colorspace conversion query and actual `VideoProcessorBlt` results for
BT.709 full/limited range with legacy/explicit color-space API calls. It uses the
same adapter-selection/device helper as the client. Errors retain stage/HRESULT.

The input is a synthetic 1920x1080 Y410 texture with a 1024-step grayscale ramp
and alternating per-pixel chroma. Output is RGB10 in one VideoProcessor pass.
**Only this offline test** copies that synthetic result to CPU-readable staging
memory to check pixel error, grayscale levels and chroma preservation. No live
or remote pixels are read. No host negotiation, decoder or real-time latency is
tested here, and successful conversion does not establish direct scanout.

A completed test means the report was written, not that every GPU case passed.
A successful blit alone is insufficient: inspect `pixels_match` and
`more_than_8bit_levels` as well. More than 256 levels is a diagnostic precision
check, not proof of the monitor's effective output depth. Query/Blt disagreement
is evidence to investigate, not permission to bypass all capability checks.

References (behavioral comparison only, no Chromium code copied):

- [Chromium texture selection](https://chromium.googlesource.com/chromium/src/+/main/media/gpu/windows/d3d11_texture_selector.cc)
- [Chromium processor adapter](https://chromium.googlesource.com/chromium/src/+/main/media/gpu/windows/d3d11_video_processor_proxy.cc)
- [Microsoft conversion query](https://learn.microsoft.com/en-us/windows/win32/api/d3d11_1/nf-d3d11_1-id3d11videoprocessorenumerator1-checkvideoprocessorformatconversion)

The original GPU-resident live conversion/rendering and its fallbacks remain
unchanged until the affected hardware has produced a conclusive report.
