@echo off
setlocal
cd /d "%~dp0"
echo ABSOLUTE HID MAPPING TEST - nine fixed positions, NO clicks.
echo Start Start-Host-Absolute-Mapping-Observer.cmd on your own host FIRST.
echo Enable host USB/Yubikey passthrough; end earlier probe sessions.
echo This test may move the host pointer on a DIFFERENT monitor from the stream.
echo Do not move the mouse during the sequence. Local menus and keyboard remain available.
echo Normal remote Parsec mouse commands are blocked for this process.
echo Connect using experimental BUD and leave connected for at least 20 seconds.
echo The sequence starts 1.5 seconds after the first HID read, then moves once per second.
pause
set "PARSEC_USB_MOUSE_PROBE=1"
set "PARSEC_USB_MOUSE_INPUT_PROBE="
set "PARSEC_USB_MOUSE_LIVE_PROBE="
set "PARSEC_USB_MOUSE_ABSOLUTE_PROBE=1"
set "PARSEC_USB_PROBE_LOG=%~dp0usb-client-absolute-%RANDOM%-%RANDOM%.jsonl"
echo Client log: %PARSEC_USB_PROBE_LOG%
"%~dp0ParsecWebTurn-USB-Probe.exe"
pause
