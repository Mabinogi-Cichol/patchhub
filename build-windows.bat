@echo off
setlocal EnableExtensions DisableDelayedExpansion
set "REPO_DIR=%~dp0"
set "PAUSE_AT_END=1"
:parse_args
if "%~1"=="" goto build
if /i "%~1"=="--no-pause" (
    set "PAUSE_AT_END=0"
    shift
    goto parse_args
)
if /i "%~1"=="--help" (
    set "SHOW_HELP=1"
    shift
    goto parse_args
)
echo Unknown option: %~1
echo Usage: build-windows.bat [--no-pause] [--help]
set "RESULT=2"
goto finish

:build
if defined SHOW_HELP goto help
pushd "%REPO_DIR%"
if errorlevel 1 (
    set "RESULT=1"
    goto finish
)
set "PUSHED=1"
set "CARGO_TARGET_DIR=%CD%\target"
echo Building a local preview without tests or release packaging...
call pwsh -NoProfile -File scripts/build-windows-flutter.ps1 -Preview
set "RESULT=%errorlevel%"
if not "%RESULT%"=="0" goto build_failed
echo Starting the preview...
start "" "%CD%\apps\windows-flutter\build\windows\x64\runner\Release\patchhub.exe"
set "RESULT=%errorlevel%"
goto finish

:build_failed
where /q pwsh
if errorlevel 1 (
    >&2 echo Install PowerShell 7 and make pwsh available on PATH, then open a new terminal.
    goto finish
)
if not exist ".tools\flutter\bin\flutter.bat" (
    where /q flutter
    if errorlevel 1 >&2 echo Set up Flutter in .tools\flutter or add its bin directory to PATH. See docs\features\platform-build-entrypoints\build-guide.md.
)
goto finish

:help
echo Builds and starts a Windows preview using incremental Rust and Flutter builds.
echo Skips tests, analysis, installers, ZIPs, SBOM, and release checksums.
echo For full release packaging, use build.bat.
echo Usage: build-windows.bat [--no-pause] [--help]
set "RESULT=0"

:finish
if defined PUSHED popd
if "%PAUSE_AT_END%"=="1" pause
exit /b %RESULT%
