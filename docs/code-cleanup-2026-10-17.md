# Dev code/dependency cleanup — 2026-10-17

## Removed

- The standalone TLS comparison crate, its lockfile and dedicated CI workflow.
  Native-tls/OpenSSL were never runtime dependencies of the application; the
  deleted experimental crate was their only direct introduction. The accepted
  rustls provider, DTLS verification and native TCP implementation remain.
- Direct D3D11 H.264 decoding left from the vendor comparison. Every application
  call constructed this decoder with HEVC; H.264 selection already routes to
  Media Foundation. Removed the unused H.264 parser allocation, DXVA parameter,
  matrix and slice staging, profile selection and submission branches. HEVC
  entries now use their own documented one-byte DXVA type without transmuting
  from a H.264 entry. A bit-layout regression checks equivalent reference flags.
- The GPU crate's now-unused direct lowlat-core dependency: its only use was the
  generic codec selector removed above. BUD still retains its required protocol
  core; the GPU crate keeps the required decode parsers and driver bindings.
- An explicitly requested but unused Win32_System_Ole feature from the root
  Windows API declaration dependency. Required COM and VARIANT features remain.
- Production compilation of Audio Pipeline::start, a default-budget convenience
  wrapper used only by tests. Live callers still use start_with_budget.

## Retained and checked

- Current direct runtime dependencies have use sites: HTTP/WSS and WebRTC/TURN,
  BUD protocol/crypto, native WASM runtime, Opus, protected settings, UI graphics,
  GPU parsers/driver bindings and self_cell ownership. No arbitrary package
  replacement, new runtime dependency or version update is part of cleanup.
- Wasmtime's build-time compiler is required to produce the embedded AOT core;
  it is not an unused runtime decoder or an abandoned alternate WASM core.
- Diagnostic commands and fixtures that validate retained features stay behind
  test/diagnostics configuration. The normal executable excludes them. Security,
  transport renewal, partial-write/cancellation, media recovery and GPU surface
  ownership regressions remain because they exercise current behavior.
- Research reports and measurements remain as historical evidence. The removed
  TLS source is linked to its historical commit, not a missing local directory.
- Proprietary WASM, live GPU surface/copy behavior, WebRTC connection policy,
  BUD routing, audio settings and user-facing features are unchanged.

The caller audit, dependency tree, strict warnings and Windows/Linux CI are used
to verify the retained code; source cleanup is not a claim of measured latency
or binary-size improvement.
