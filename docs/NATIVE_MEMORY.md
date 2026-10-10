# Native client memory investigation

Scope: baseline application memory, not the native video decoder, texture pool,
presentation queue or audio buffers. The user's observed baseline is about
200 MB, with roughly 50 MB added during a stream. These observations are not
attributed to individual components without measurement.

## Current architecture

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

## Isolated comparison

The `diagnostics` build generates an AOT artifact with the same locked Wasmtime
version and execution configuration, using a portable target rather than CPU
features detected on the build machine. Only that immutable embedded artifact
is deserialized. User-supplied serialized native code is never accepted. The
normal client's loading path remains unchanged for this investigation.

Run separate fresh processes on the same machine, alternating order and
repeating each path:

```powershell
./parsec-native-memory-diagnostics.exe window-memory-jit ./parsecd.wasm ./jit.json
./parsec-native-memory-diagnostics.exe window-memory-aot ./parsecd.wasm ./aot.json
```

Both render the same offline login UI for a bounded interval, verify the guest
allocator and cleanly shut down. Neither loads the user's profile or enables
account networking. The WASM path is hash-checked even in AOT mode; it is never
treated as a serialized native artifact.

CI has no accelerated OpenGL pixel format. It uses the separate
`allocator-memory-jit` and `allocator-memory-aot` modes to validate loading and
the real WASM allocator without a window. Those are not UI memory measurements.

Reports include module loading time, process working set and private committed
bytes before/after loading, bounded samples while rendering, current guest
linear-memory size, and CPU atlas/buffer/frame-vertex byte counts. Working set
includes shared pages; private commit includes memory not currently resident.
Neither should be mislabeled as Task Manager's active private working set.

UI byte counts cover retained payloads, not allocator overhead, OpenGL driver
allocations, GPU residency or the WASM guest's original image data. Fresh-process
AOT/JIT comparisons isolate the effect of executing compilation; both diagnostic
paths still link the compiler. A later runtime-only production build could omit
it entirely, but that saving is not established by this comparison alone.

## Decision criteria

- Prefer AOT if the measured improvement justifies its embedded native-code
  size and build-time compatibility requirements. Do not weaken fuel, epoch,
  shared-memory or host-call validation to reduce memory.
- Optimize atlas representation or lifecycle only if actual retained atlas
  bytes justify it. Preserve partial updates and the overlay; do not add live
  GPU readback or change native video buffers.
- Do not use working-set trimming to manufacture a smaller Task Manager number;
  paging the same data back in can hurt responsiveness.

## References

- [Wasmtime pre-compilation](https://docs.wasmtime.dev/examples-pre-compiling-wasm.html)
- [Wasmtime module deserialization and safety](https://docs.wasmtime.dev/api/wasmtime/struct.Module.html#method.deserialize)
- [Windows process working set](https://learn.microsoft.com/en-us/windows/win32/procthread/process-working-set)
