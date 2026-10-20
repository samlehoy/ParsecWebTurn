@echo off
setlocal
cd /d "%~dp0"
echo LIVE DESKTOP HID MOUSE TEST - 90 seconds per process.
echo Enable USB/Yubikey passthrough on your own host. Close earlier probe sessions.
echo Run the NEW host observer, then connect using experimental BUD.
echo Mouse movement, five buttons and wheels use ONLY the test HID mouse.
echo Normal Parsec mouse injection is blocked; keyboard and local menus still work.
echo Test on an empty Paint canvas and a scrollable desktop document.
echo Focus another app to release buttons. Disconnect to end the test early.
echo After 90 seconds the test mouse is removed; restart the client for a new test.
echo Normal mouse injection will NOT automatically resume when the test ends.
echo Relative HID input may differ from absolute pointer positioning.
pause
set "PARSEC_USB_MOUSE_PROBE=1"
set "PARSEC_USB_MOUSE_INPUT_PROBE="
set "PARSEC_USB_MOUSE_ABSOLUTE_PROBE="
set "PARSEC_USB_MOUSE_LIVE_PROBE=1"
set "PARSEC_USB_PROBE_LOG=%~dp0usb-client-live-%RANDOM%-%RANDOM%.jsonl"
echo Client log: %PARSEC_USB_PROBE_LOG%
"%~dp0ParsecWebTurn-USB-Probe.exe"
pause
