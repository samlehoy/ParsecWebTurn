@echo off
setlocal
cd /d "%~dp0"
echo Read-only absolute HID observation: desktop bounds and grid matches only.
echo No mouse hooks, injection, driver changes or administrator rights.
"%~dp0USB-Mouse-Observer.exe" "%~dp0usb-host-absolute-%RANDOM%-%RANDOM%.jsonl" --absolute-mapping
pause
