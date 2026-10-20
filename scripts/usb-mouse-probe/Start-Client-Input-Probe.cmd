@echo off
setlocal
cd /d "%~dp0"
echo DESKTOP TEST: this sends small right/left movement and ONE LEFT CLICK.
echo On the host open a blank Paint canvas and put the cursor in its center.
echo Keep games and other sensitive windows closed. Do not touch the mouse during the sequence.
echo The sequence starts after the host requests interrupt input, and lasts about 7 seconds.
echo Keep the connection open for 25 seconds. Host USB/Yubikey passthrough must be enabled.
pause
set "PARSEC_USB_MOUSE_PROBE=1"
set "PARSEC_USB_MOUSE_INPUT_PROBE=1"
set "PARSEC_USB_PROBE_LOG=%~dp0usb-client-input-%RANDOM%-%RANDOM%.jsonl"
echo Connect using experimental BUD. Only the first established session is probed.
echo Client log: %PARSEC_USB_PROBE_LOG%
"%~dp0ParsecWebTurn-USB-Probe.exe"
pause
