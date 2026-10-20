@echo off
setlocal
cd /d "%~dp0"
echo LIVE ABSOLUTE HID MOUSE TEST - 90 seconds, single physical or virtual display.
echo Enable host USB/Yubikey passthrough; close earlier probe sessions.
echo Run the NEW host observer, then connect using experimental BUD.
echo Test alignment in Paint, click, drag/release, five buttons and both wheels.
echo Mouse input uses ONLY the test HID mouse; local menus and keyboard keep working.
echo This is a desktop absolute-pointer experiment, not a relative mouselook mode.
echo Focus another app or open local Settings to release remote buttons.
echo After 90 seconds the device is removed. Restart for another test.
echo Normal Parsec mouse injection does NOT resume automatically on expiry/failure.
pause
set "PARSEC_USB_MOUSE_PROBE=1"
set "PARSEC_USB_MOUSE_INPUT_PROBE="
set "PARSEC_USB_MOUSE_ABSOLUTE_PROBE="
set "PARSEC_USB_MOUSE_LIVE_PROBE=1"
set "PARSEC_USB_MOUSE_LIVE_ABSOLUTE_PROBE=1"
set "PARSEC_USB_PROBE_LOG=%~dp0usb-client-live-absolute-%RANDOM%-%RANDOM%.jsonl"
echo Client log: %PARSEC_USB_PROBE_LOG%
"%~dp0ParsecWebTurn-USB-Probe.exe"
pause
