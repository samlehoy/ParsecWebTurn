@echo off
setlocal
cd /d "%~dp0"
set "tools=%~dp0mftrace-tools"
set "probe=%~dp0intel-hevc-paths.exe"
if not exist "%probe%" (
    echo Extract this launcher beside intel-hevc-paths.exe.
    goto failed
)
if /i "%~1"=="/prepare" goto prepare_tools
"%SystemRoot%\System32\fltmc.exe" >nul 2>&1
if not errorlevel 1 (
    echo Start this launcher normally, under your ordinary Windows account.
    echo It will create a SEPARATE attach helper for administrator use.
    echo Starting the test itself as another admin can hide per-user codecs.
    goto failed
)

:prepare_tools
if not exist "%tools%" mkdir "%tools%"
if errorlevel 1 goto failed
if not exist "%tools%\mftrace.exe" goto extract_tools
if not exist "%tools%\mfdetours.dll" goto extract_tools
goto verify_tools

:extract_tools
if exist "%tools%\sdk-tools.cab" goto verify_cab
echo Downloading original Microsoft MFTrace tools, 21 MB, first run only.
echo No SDK or codec will be installed. No PowerShell or policy change is used.
"%SystemRoot%\System32\curl.exe" --fail --location --proto =https --tlsv1.2 --output "%tools%\sdk-tools.cab" "https://download.visualstudio.microsoft.com/download/pr/c001d4bb-6b32-4ca0-896e-7e0e82df71eb/36b6c5488d0c568b743ac806fd533be4/69661e20556b3ca9456b946c2c881ddd.cab"
if errorlevel 1 goto failed
:verify_cab
call :hash "%tools%\sdk-tools.cab" EE19778F3B0D943D1D18A33D03EC26F2EFE0D4E3BB5CE91C2D10E810F4392F80
if errorlevel 1 goto failed
"%SystemRoot%\System32\expand.exe" "%tools%\sdk-tools.cab" -F:fil1c47c0ccebc004a02d80c59453d91adb "%tools%"
if errorlevel 1 goto failed
"%SystemRoot%\System32\expand.exe" "%tools%\sdk-tools.cab" -F:fil0faecef52df811576e865cc6d05ba3c9 "%tools%"
if errorlevel 1 goto failed
move /y "%tools%\fil1c47c0ccebc004a02d80c59453d91adb" "%tools%\mftrace.exe" >nul
if errorlevel 1 goto failed
move /y "%tools%\fil0faecef52df811576e865cc6d05ba3c9" "%tools%\mfdetours.dll" >nul
if errorlevel 1 goto failed

:verify_tools
call :hash "%tools%\mftrace.exe" 6C3F5B155721DED42A7B3BCA459EE1D8D0433081C93E2011C64BBB5C91F914A2
if errorlevel 1 goto failed
call :hash "%tools%\mfdetours.dll" 103C40D6C5B75EB080CC8E343661E90B60416C713DA8281BBA0E2B21F09DEF17
if errorlevel 1 goto failed
if /i "%~1"=="/prepare" (
    echo Microsoft tools ready. No trace was started.
    exit /b 0
)

:choose_folder
set "results=intel-hevc-trace-%RANDOM%-%RANDOM%"
if exist "%results%" goto choose_folder
if exist "%results%.zip" goto choose_folder
mkdir "%results%"
if errorlevel 1 goto failed
copy /y "%probe%" "%results%\intel-hevc-paths.exe" >nul
if errorlevel 1 goto failed
"%SystemRoot%\System32\certutil.exe" -hashfile "%probe%" SHA256 >"%results%\probe-sha256.txt" 2>&1
if errorlevel 1 goto failed
if exist "%~dp0SOURCE.txt" copy /y "%~dp0SOURCE.txt" "%results%\SOURCE.txt" >nul
set "control="
if /i "%~1"=="/control" set "control=--control"
echo Testing under your ordinary account. Leave this console open.
echo The test will print the attach helper path before codec discovery starts.
pushd "%results%"
intel-hevc-paths.exe --wait-for-trace %control%
set "trace_exit=%ERRORLEVEL%"
popd
if not "%trace_exit%"=="0" goto failed
echo Stop the ADMIN MFTrace window with Ctrl+C if it is still running.
echo Then press a key here to package the completed logs.
pause
if not exist "%results%\intel-hevc-paths.json" goto failed
if not exist "%results%\intel-hevc-mftrace.txt" goto failed
for %%F in ("%results%\intel-hevc-mftrace.txt") do if %%~zF==0 goto failed
pushd "%results%"
"%SystemRoot%\System32\tar.exe" -a -cf "..\%results%.zip" intel-hevc-mftrace.txt intel-hevc-paths.json intel-hevc-console.txt probe-sha256.txt
set "archive_exit=%ERRORLEVEL%"
popd
if not "%archive_exit%"=="0" goto failed
echo Send this archive: %~dp0%results%.zip
pause
exit /b 0

:hash
"%SystemRoot%\System32\certutil.exe" -hashfile "%~1" SHA256 >"%tools%\hash-check.txt" 2>&1
if errorlevel 1 exit /b 1
"%SystemRoot%\System32\findstr.exe" /i /x /c:"%~2" "%tools%\hash-check.txt" >nul
if errorlevel 1 (
    echo Hash mismatch. Do not use these diagnostic tools: %~1
    exit /b 1
)
exit /b 0

:failed
echo Diagnostic could not complete. Keep the error message and result folder.
if /i "%~1"=="/prepare" exit /b 1
pause
exit /b 1
