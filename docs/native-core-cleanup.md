# Native build cleanup — 2026-10-24

Scope: `codex/native-core`; preserve the normal client and every implemented
native media/input/transport feature. No decoder/presentation algorithm changes.

## Removed from native release compilation

- Optional Wasmtime runtime/AOT compiler and Cranelift, including the build-script
  compiler dependency. No core file, guest instantiation, guest filesystem/HTTP/
  WebSocket dispatch, guest worker or fuel/watchdog adapter is built for release.
- glow/GLES guest UI, its software GUI command capture/rasterizer, and the
  associated unused window buffers, reports and hotkey binding state.
- Guest-memory audio polling and cursor/input ABI readers. Native PCM handoff,
  native input/cursor ownership and the existing decoder workers remain unchanged.

The legacy modules stay available to the normal default `wasm-core` build and to
synthetic ABI regression tests. Deleting those tests would remove useful shared
transport/input/cancellation coverage. Native release dependencies are checked
separately with Cargo's `normal,build` edges, excluding test dependencies.

## Shared boundaries simplified

- BUD offer completion uses a callback only when a legacy guest exists; the native
  account path has no guest-memory output object and allocates no such callback.
- Video protocol metadata lives with video-stream parsing rather than the old
  guest backend. The unchanged 1 MiB channel limit has one common definition.
- All three native host-video update triggers share serialization, sending,
  logging and confirmation setup. There is no change to video frame processing.
- Menu shortcuts are dispatched in the UI thread; the redundant old KeyO branch
  in the native session loop is removed.
- Application icon loading shares one embedded resource. No downloaded icon,
  image-generation library or runtime asset loader is added.

Removing Wasmtime also exposed Opus's upstream CMake default `/MD` runtime
overriding Rust's configured static CRT. The MSVC-target-only CMake toolchain
sets upstream `OPUS_STATIC_RUNTIME=ON`; no codec source, DSP option or audio path
is changed and no replacement DLL/shim is introduced.

## Retained lowlat runtime packages

| Package | Required role |
| --- | --- |
| lowlat-client | BUD session control, bounded media handoff, USB HID |
| lowlat-core | BUD framing/reliability and session state |
| lowlat-net | Native UDP/TCP wait integration and TURN stream framing |
| lowlat-crypto | Session crypto/DTLS required by the stock host |
| lowlat-common | Shared synchronization/logging primitives |
| lowlat-decode | Existing vendor decoder integration |
| lowlat-drivers | Existing vendor/Windows GPU API bindings |

No lowlat SDK, daemon, host, encoder, capture, installer or UI dependency is
included. Existing licenses and pinned provenance remain. Crypto/TLS certificate
validation, GPU decoder surfaces, conversion/presentation, queues and camera/HID
semantics are not replaced. Shared WebRTC/ICE types are still needed by connection
settings/policy; removing them requires a separate type-boundary change rather
than deleting a seemingly unused dependency.

## Ctrl+Alt+Del investigation

The supplied client log records successful sub-ID 14 sends. Static inspection of
the installed stock host confirms that owner-only sub-ID 14 calls its `SAS` named
pipe service request. The user also confirmed failure with the stock client and
a portable host. This is not evidence of incorrect native wire framing. Keep the
host-authorized request; do not substitute ordinary injected keys or bypass the
host service. A send event is not an execution acknowledgement.
