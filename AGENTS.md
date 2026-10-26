# Development requirements

The user's standing requirement is to keep current and future development standards-compliant and maintainable.

- Use W3C/IETF standards for WebRTC, ICE, STUN and TURN behavior. Preserve native WebIDL validation and the client's connection policy.
- Use documented, supported Rust and target-platform APIs for platform functionality (currently Windows; Linux/Android when added). Platform APIs are not automatically web standards; describe that distinction accurately. This branch contains the independent EvoParseq native Rust client.
- Do not introduce production dependencies on private Chromium interfaces, internal browser pages or undocumented switches. The user permits documented vendor diagnostics as an optional extension: keep them disabled by default, clearly label their experimental/vendor status, isolate failures, and retain a standards-based fallback. Never use unstable diagnostic output as a required production contract or a definitive routing proof.
- Missing telemetry is unknown, not negative evidence. Report confirmed direct/TURN routing only when the selected active connection provides sufficient evidence; preserve uncertainty otherwise.
- Keep diagnostic tests isolated from real credentials and profiles. Do not expose production remote debugging ports, weaken browser security/privacy or require administrator rights for ordinary use.
- For each proposed integration, verify its documented support and fallback behavior before implementation. If a required result cannot be established through supported APIs, document the limitation instead of inventing a heuristic presented as certainty.
- For Parsec protocol research, consult `docs/NATIVE_REFERENCES.md`; it records external projects and the limits of applying their findings to this client.

- Native video must use documented codec and graphics APIs. Keep live decoded frames on the GPU, feed decoder textures directly to the presentation/conversion pass, and avoid unnecessary intermediate decoded-frame copies or CPU readback. Necessary color conversion/output writes or documented implementation/interop copies are permitted; explain why they are needed and do not advertise them as zero-copy. Synthetic offline diagnostics may perform explicitly isolated readback; never enable it in live sessions. Do not claim zero-copy or hardware execution beyond what the implementation and telemetry establish.

- BUD is authorized only as an explicit alternative. WebRTC remains the default. BUD integration must not alter the existing WebRTC connection/signaling, ICE/STUN/TURN selection, transport timers, SCTP reliability/ordering or settings behavior. Isolate the BUD adapter, credentials and lifecycle; use only deliberate, reviewed common interfaces. A BUD failure must not silently fall back or change the selected transport. Preserve the working WebRTC TURN/TCP and TURN/TLS routes, especially for restricted networks. Validate both transports independently before exposing a usable BUD option.

## UI and platform boundaries for future development

The user requires continuing the incremental separation established on the native-core branch:

- Put application state, parsing/validation rules, option dependencies, persistence decisions and presentation text/units in shared Rust models/controllers. Reuse the existing models; do not duplicate their rules in each UI.
- Keep native widget access, drawing, focus/capture, platform input translation, storage protection and OS/media API operations in explicit adapters/backends. Shared events and commands cross the boundary without an extra event queue or payload-conversion layer.
- Add only interfaces required by implemented behavior. Prefer static dispatch and existing services; do not introduce speculative frameworks, abstractions, SDKs or dependencies just to prepare for another platform.
- Preserve the working decoding/presentation/audio paths, ordering, bounds and wakeups. Architectural separation must not add decoded-frame copies, CPU readback, per-frame work, unnecessary buffers, threads or timers. Keep CPU and memory use low; do not claim a speedup without measurement.
- Preserve and test focus-loss releases, immersive capture, relative camera transitions, HID synchronization and clipboard permissions/echo suppression when modifying these boundaries. Use isolated fixtures and the existing native lifecycle tests, followed by live user validation for hardware/session behavior.
- Review each completed step for duplicated policy, unused helpers and incomplete experiments. Retain necessary protocol validation and platform backends; do not remove them merely because a future port will replace them. Port actual media/storage backends when a concrete target platform requires it.
