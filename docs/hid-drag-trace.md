# Temporary HID drag diagnostics

This test branch changes no mouse/control behavior. The ineffective endpoint
replay candidate has been reverted. The normal build has no trace code enabled;
only this branch's `hid-input-trace` test executable records mouse diagnostics.
No new dependency is added. No video/GPU path is instrumented or changed.

Run the test EXE with the usual settings.json. Enable BUD HID mouse, connect,
drag a remote window once or twice, release it, then click without moving the
mouse. Close the client normally. Send `hid-drag-trace.json` from beside
settings.json. Administrator rights, host changes and helpers are not needed.

The capture contains mouse coordinates/buttons, cursor mode, the core's mouse
messages, adapter commands and mouse USB reports accepted by BUD for sending.
It records no keyboard usages, text, clipboard, credentials, addresses, media
or arbitrary USB payloads. It is separate from client-events.jsonl.

At most 16,384 numeric records are kept in memory and written once on shutdown;
the omitted count identifies saturation. Reproduce promptly after connecting.
Timing is diagnostic CPU arrival order, not host receipt/completion proof.
The temporary tracing branch is not intended for merging into dev.
