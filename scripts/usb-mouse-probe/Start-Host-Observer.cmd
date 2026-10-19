@echo off
setlocal
cd /d "%~dp0"
"%~dp0USB-Mouse-Observer.exe" "%~dp0usb-host-%RANDOM%-%RANDOM%.jsonl"
pause
