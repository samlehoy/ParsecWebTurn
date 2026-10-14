# Native client memory investigation

Scope: baseline application memory, not the native video decoder, texture pool,
presentation queue or audio buffers. The user's observed baseline is about
200 MB, with roughly 50 MB added during a stream. These observations are not
attributed to individual components without measurement.

## Architecture before the investigation

The normal EXE embeds the SHA-256-pinned Parsec WASM; it does not fetch it at
startup. Wasmtime compiles it locally on each launch. The imported shared linear
memory has an initial 174 pages (10.875 MiB), maximum 16,384 pages (1 GiB).
The maximum is a limit, not proof that 1 GiB of physical RAM is used. The guest
can grow its memory while initializing its fonts, account UI and session.

The account UI uses native OpenGL. The overlay adapter retains CPU copies of UI
atlases and vertex buffers so it can reproduce that UI over native D3D11 video.
Texture references are shared with `Arc`, not copied per batch. A partial update
can create a copy when an older frame still references the previous version.
The D3D11 overlay cache already removes obsolete atlas versions after drawing.
Deleting the CPU atlases unconditionally would break subsequent partial updates
or the overlay. Readback from OpenGL is not a suitable replacement.

## Implemented scope

The normal client now embeds the build-generated AOT artifact and loads it
directly, without a runtime compiler. Only the hash-verified original core is
compiled, with the locked Wasmtime version, shared execution settings and a
portable target. Fuel and epoch limits are preserved and tested. No external
serialized native code is accepted.

The temporary memory comparison commands, counters and workflow artifacts were
removed after the investigation. Existing isolated allocator/bootstrap probes
now exercise the same embedded AOT loader as the normal client. The measurements
below describe the historical diagnostic build, which still linked a compiler.
No UI texture, video or audio optimization was adopted from this investigation.

## Local results (2026-10-10)

Measured the optimized diagnostic executable from `3b5f57e` on the local AMD
Radeon 780M system. Eight fresh offline UI processes were run, alternating order:
four JIT, four AOT. Every run rendered 14 UI frames, reported no execution error,
and released its native window. No real account, clipboard or Parsec host was
used. The AOT artifact uses the portable Windows target; the JIT path retains
the normal host CPU configuration. This is not a throughput or latency benchmark.

- Core loading: JIT 1,145–1,218 ms; AOT 3–5 ms. Timing excludes file reading and
  hashing and includes creating the engine and loading/compiling the module.
- Guest linear-memory extent after UI initialization: **106.0625 MiB** for both
  loading paths. This is accessible linear-memory size, not a measurement of
  how many of its pages are physically resident.
- Retained overlay CPU payload: three textures, **20,141,664 bytes (19.21 MiB)**;
  buffers 19,528 bytes; frame vertices 46,440 bytes. Texture references shared
  between batches are counted once.
- Immediately after core loading in the first four runs, private commit was
  20.83–22.09 MiB for JIT and 12.89–12.90 MiB for AOT. That modest difference did
  not establish an equivalent reduction in the fully initialized UI.

The UI is event driven and presented only 14 frames, so sampling once per 60
frames does not provide steady-state coverage. Four additional runs used an
external process counter every 250 ms and averaged samples between seconds 4
and 7. Each row contains 11 samples; raw counters are preserved alongside the
local JSON reports in `dist/native-memory-investigation/` (ignored build data).

| Loading path / order | Private commit, MiB | Total working set, MiB |
| --- | ---: | ---: |
| AOT / 1 | 396.80 | 288.52 |
| JIT / 2 | 398.21 | 278.20 |
| JIT / 3 | 383.71 | 257.57 |
| AOT / 4 | 379.08 | 258.65 |

Run-to-run variation is larger than the observed difference between loading
paths. These values include driver/runtime allocations and are not Task
Manager's private working set. They do **not** prove a material steady-state
RAM reduction from AOT. The local build still links the compiler in both modes;
the normal binary now omits it, but no memory-saving claim is made.

Conclusion: AOT is strongly supported as a startup improvement, not as a
demonstrated solution to the roughly 200 MB baseline. Atlas storage has a
measurable 19.21 MiB payload worth examining for compact formats, but it cannot
account for the entire baseline. Only AOT loading was adopted; native video resources were not changed.

## References

- [Wasmtime pre-compilation](https://docs.wasmtime.dev/examples-pre-compiling-wasm.html)
- [Wasmtime module deserialization and safety](https://docs.wasmtime.dev/api/wasmtime/struct.Module.html#method.deserialize)
- [Windows process working set](https://learn.microsoft.com/en-us/windows/win32/procthread/process-working-set)
