@echo off
cd /d "%~dp0"
echo Intel HEVC comparison. Run as your normal Windows user.
echo This takes approximately one minute. A test window may open at the end.
intel-hevc-compare.exe > intel-hevc-console.txt 2>&1
if errorlevel 1 (
  echo Test failed. Please send intel-hevc-console.txt.
) else (
  echo Finished. Please send intel-hevc-comparison.json and intel-hevc-console.txt.
)
pause
