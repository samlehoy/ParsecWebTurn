@echo off
setlocal DisableDelayedExpansion
cd /d "%~dp0"
if not exist intel-hevc-paths.exe goto failed
if not exist Intel-HEVC-ETW.wprp goto failed
if not exist "%SystemRoot%\System32\wpr.exe" goto failed
"%SystemRoot%\System32\fltmc.exe" >nul 2>&1
if not errorlevel 1 (
    echo Start THIS launcher normally, under your ordinary Windows account.
    echo Only the separate RECORDING helper needs administrator rights.
    goto failed
)
:choose_folder
set "results=intel-hevc-gpu-trace-%RANDOM%-%RANDOM%"
if exist "%results%" goto choose_folder
if exist "%results%.zip" goto choose_folder
mkdir "%results%"
if errorlevel 1 goto failed
copy /y intel-hevc-paths.exe "%results%\intel-hevc-paths.exe" >nul
if errorlevel 1 goto failed
"%SystemRoot%\System32\certutil.exe" -hashfile intel-hevc-paths.exe SHA256 >"%results%\probe-sha256.txt" 2>&1
if errorlevel 1 goto failed
set "control="
if /i "%~1"=="/control" set "control=--control"
echo GPU and HEVC decoder event trace. No PowerShell or tool download is used.
echo Close other video players and the live client before starting.
pushd "%results%"
intel-hevc-paths.exe --wait-for-gpu-trace %control%
set "test_exit=%ERRORLEVEL%"
popd
if not "%test_exit%"=="0" goto failed
echo Return to the ADMIN recording window and press a key to SAVE its trace.
echo Wait for 'Recording saved', then press a key HERE to package the logs.
pause
if not exist "%results%\intel-hevc-gpu.etl" goto failed
if not exist "%results%\intel-hevc-paths.json" goto failed
for %%F in ("%results%\intel-hevc-gpu.etl") do if %%~zF==0 goto failed
pushd "%results%"
"%SystemRoot%\System32\tar.exe" -a -cf "..\%results%.zip" intel-hevc-gpu.etl intel-hevc-paths.json intel-hevc-console.txt probe-sha256.txt trace-target.txt wpr-start.txt wpr-stop.txt
set "archive_exit=%ERRORLEVEL%"
popd
if not "%archive_exit%"=="0" goto failed
echo Send this archive: %~dp0%results%.zip
pause
exit /b 0
:failed
echo Diagnostic did not complete. Keep the error message and result folder.
echo If a recording was started, use its generated SAVE helper as administrator.
pause
exit /b 1
