@echo off
setlocal
cd /d "%~dp0"
echo Running Intel VPL GPU probe...
intel-gpu-probe.exe > intel-gpu-diagnostics.json
if errorlevel 1 goto failed
echo Running offline Media Foundation baseline, please wait...
mf-baseline-probe.exe > mf-baseline-diagnostics.json
if errorlevel 1 goto failed
echo Both reports saved beside this launcher.
echo intel-gpu-diagnostics.json
echo mf-baseline-diagnostics.json
pause
exit /b 0
:failed
echo Probe failed. Please send the console error.
pause
exit /b 1
