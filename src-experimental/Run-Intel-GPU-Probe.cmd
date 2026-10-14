@echo off
setlocal
cd /d "%~dp0"
intel-gpu-probe.exe > intel-gpu-diagnostics.json
if errorlevel 1 (
  echo The Intel GPU probe failed. Please send the console error.
) else (
  echo Results saved to intel-gpu-diagnostics.json beside this launcher.
  type intel-gpu-diagnostics.json
)
pause
