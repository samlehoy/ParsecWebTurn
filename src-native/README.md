# Native Rust client

The application crate lives here. See [the root README](../README.md) for supported features, persistent profile behavior, build instructions and limitations.

The normal build embeds the audited Parsec WASM core. Run `./src-native/fetch-core.ps1` from the repository root before compiling. A changed upstream hash fails closed and requires a new ABI audit.

The build precompiles that pinned core with the locked Wasmtime version for the portable target. The resulting native-code artifact is embedded in the single EXE; the normal runtime loads only this immutable artifact and does not include the WASM compiler. Fuel, epoch interruption and shared-memory settings remain unchanged. Rebuild the artifact whenever Wasmtime or its configuration changes; external compiled caches are never loaded.

`diagnostics` is an opt-in build feature for offline probes and prototype reports. It is excluded from the distributed executable. Test fixtures under `fixtures` are used only by tests and diagnostic builds. The pinned Parsec ABI is a private integration contract; Windows decoding/presentation and WebRTC transport use documented APIs and protocols.

The vendored WebRTC dependency and its narrowly scoped compatibility changes are documented in [LOCAL-CHANGES.md](vendor/webrtc-0.14.0/LOCAL-CHANGES.md).

Native STUN/TURN settings use the [legacy-compatible configuration contract](../docs/NATIVE_CONFIG_COMPATIBILITY.md).
TURN UDP/TCP/TLS integration is documented in the [ICE patch notes](vendor/webrtc-ice-0.14.0/LOCAL-CHANGES.md).
