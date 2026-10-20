@echo off
setlocal
cd /d "%~dp0"
set "PARSEC_USB_MOUSE_PROBE=1"
set "PARSEC_USB_MOUSE_INPUT_PROBE="
set "PARSEC_USB_MOUSE_LIVE_PROBE="
set "PARSEC_USB_MOUSE_ABSOLUTE_PROBE="
set "PARSEC_USB_MOUSE_LIVE_ABSOLUTE_PROBE="
set "PARSEC_USB_PROBE_LOG=%~dp0usb-client-%RANDOM%-%RANDOM%.jsonl"
echo Connect to your own host using experimental BUD.
echo Only the first established BUD session is probed in this process.
echo The mouse is announced after 2 seconds and removed after 17 seconds.
echo No cursor movement or clicks are sent. Keep the connection open for 25 seconds.
echo Client log: %PARSEC_USB_PROBE_LOG%
"%~dp0ParsecWebTurn-USB-Probe.exe"
pause
