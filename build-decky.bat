@echo off
setlocal EnableExtensions DisableDelayedExpansion
set "REPO_DIR=%~dp0"

set "PAUSE_AT_END=1"
:parse_args
if "%~1"=="" goto args_done
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
echo Usage: build-decky.bat [--no-pause] [--help]
set "RESULT=2"
goto finish

:args_done
if defined SHOW_HELP goto help
pushd "%REPO_DIR%"
if errorlevel 1 (
    echo ERROR: Could not enter the repository directory: %REPO_DIR%
    set "RESULT=1"
    goto finish
)
set "PUSHED=1"

where node >nul 2>&1
if errorlevel 1 goto missing_node
where npm >nul 2>&1
if errorlevel 1 goto missing_node
where wsl.exe >nul 2>&1
if errorlevel 1 (
    echo ERROR: WSL with a configured Linux distribution is required.
    set "RESULT=1"
    goto finish
)

set "PYTHON="
call py -3 -c "import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)" >nul 2>&1
if not errorlevel 1 set "PYTHON=py -3"
if not defined PYTHON (
    call python -c "import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)" >nul 2>&1
    if not errorlevel 1 set "PYTHON=python"
)
if not defined PYTHON (
    echo ERROR: Python 3.11 or newer is required for the release SBOM and ZIP.
    set "RESULT=1"
    goto finish
)

echo [1/5] Installing locked npm dependencies...
set "STEP=npm ci"
call npm ci
if errorlevel 1 goto build_failed

echo [2/5] Typechecking the Decky frontend...
set "STEP=npm run typecheck"
call npm run typecheck
if errorlevel 1 goto build_failed

echo [3/5] Building the Linux CLI and worker in WSL...
if not exist ".artifacts" mkdir ".artifacts"
if errorlevel 1 (
    set "STEP=creating .artifacts"
    goto build_failed
)
set "STEP=WSL Linux build"
wsl.exe --cd "%CD%" --exec sh scripts/build-linux.sh
if errorlevel 1 goto build_failed
set "STEP=WSL cargo metadata --locked"
wsl.exe --cd "%CD%" --exec env CARGO_HOME=.tools/linux-cargo RUSTUP_HOME=.tools/linux-rustup CARGO_TARGET_DIR=target/linux .tools/linux-cargo/bin/cargo metadata --locked --format-version 1 > ".artifacts\cargo-metadata.json"
if errorlevel 1 goto build_failed

echo [4/5] Building and staging the Decky plugin...
set "STEP=npm run package:plugin -w apps/decky"
call npm run package:plugin -w apps/decky
if errorlevel 1 goto build_failed

echo [5/5] Generating the SBOM, plugin ZIP, and checksums...
set "STEP=scripts/build-sbom.py"
call %PYTHON% "scripts\build-sbom.py" ".artifacts\cargo-metadata.json"
if errorlevel 1 goto build_failed
set "STEP=scripts/package-artifacts.py --platform decky"
call %PYTHON% "scripts\package-artifacts.py" --platform decky
if errorlevel 1 goto build_failed
set "STEP=copying the Decky installer"
copy /y "scripts\install-decky.py" ".artifacts\install-decky.py" >nul
if errorlevel 1 goto build_failed

echo.
echo Build complete. Release files are in .artifacts.
set "RESULT=0"
goto finish

:help
echo Builds the Decky plugin ZIP, SBOM, checksums, and installer script.
echo Requires Windows Node.js/npm, Python 3.11+, and the default WSL distribution.
echo WSL requires the Linux Rust toolchain and native build dependencies.
echo Uses scripts/build-linux.sh and its .tools/linux-cargo toolchain setup.
echo Usage: build-decky.bat [--no-pause] [--help]
set "RESULT=0"
goto finish

:missing_node
echo ERROR: Node.js and npm are required on the Windows PATH.
set "RESULT=1"
goto finish

:build_failed
set "RESULT=%errorlevel%"
echo.
echo ERROR: Build failed during %STEP% ^(exit code %RESULT%^).

:finish
if defined PUSHED popd
if "%PAUSE_AT_END%"=="1" pause
exit /b %RESULT%
