@echo off
setlocal
cd /d "%~dp0"
set "EVOPARSEQ_Y410_REPORT=%~dp0y410-probe.json"
"%~dp0EvoParseq-Y410-Probe.exe" --ignored --exact video_windows::y410_probe::compare_y410_conversion_paths --nocapture --test-threads=1 > "%~dp0y410-console.txt" 2>&1
set "probeResult=%ERRORLEVEL%"
echo Diagnostic finished with exit code %probeResult%.
echo Please share y410-probe.json and y410-console.txt.
pause
exit /b %probeResult%
