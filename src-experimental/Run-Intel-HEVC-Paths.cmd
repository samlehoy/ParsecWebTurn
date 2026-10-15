@echo off
setlocal
cd /d "%~dp0"
intel-hevc-paths.exe > intel-hevc-console.txt 2>&1
echo Results: intel-hevc-paths.json
echo Console: intel-hevc-console.txt
echo Please send both files. No Parsec account or network connection is used.
pause
