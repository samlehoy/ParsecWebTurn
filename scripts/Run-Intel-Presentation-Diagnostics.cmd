@echo off
cd /d "%~dp0"
echo Short Intel presentation test. Run as your normal Windows user.
echo First look for solid MAGENTA, then the grey/green reference pattern.
intel-hevc-compare.exe --presentation-only > intel-presentation-console.txt 2>&1
if errorlevel 1 (
  echo Test failed. Please send intel-presentation-console.txt.
) else (
  echo Finished. Please send intel-presentation-diagnostics.json and intel-presentation-console.txt.
)
pause
