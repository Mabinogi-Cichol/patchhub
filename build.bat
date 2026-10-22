@echo off
setlocal EnableExtensions DisableDelayedExpansion

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
echo Usage: build.bat [--no-pause] [--help]
set "RESULT=2"
goto finish

:help
echo Builds the Windows CLI, Flutter NSIS installer, portable ZIP, SBOM, and checksums.
echo Run build.bat by double-clicking, or use --no-pause for automation.
set "RESULT=0"
goto finish

:args_done
if defined SHOW_HELP goto help
pushd "%~dp0"
if errorlevel 1 (
    echo ERROR: Could not enter the repository directory: %~dp0
    set "RESULT=1"
    goto finish
)
set "PUSHED=1"
set "CARGO_TARGET_DIR=%CD%\target"

if exist ".tools\cargo\bin\cargo.exe" (
    set "CARGO_HOME=%CD%\.tools\cargo"
    set "RUSTUP_HOME=%CD%\.tools\rustup"
    set "PATH=%CD%\.tools\cargo\bin;%PATH%"
)

where node >nul 2>&1
if errorlevel 1 goto missing_node
where npm >nul 2>&1
if errorlevel 1 goto missing_npm
where cargo >nul 2>&1
if errorlevel 1 goto missing_rust
where rustc >nul 2>&1
if errorlevel 1 goto missing_rust

set "RUST_HOST="
for /f "delims=" %%H in ('rustc --print host-tuple 2^>nul') do set "RUST_HOST=%%H"
if not defined RUST_HOST goto missing_rust
if /i not "%RUST_HOST:~-12%"=="windows-msvc" (
    echo ERROR: A Windows MSVC Rust host is required; detected %RUST_HOST%.
    echo Install the MSVC Rust toolchain and Visual Studio C++ Build Tools.
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
    echo Install Python and make py or python available on PATH.
    set "RESULT=1"
    goto finish
)

echo [1/6] Installing locked npm dependencies...
set "STEP=npm ci"
call npm ci
if errorlevel 1 goto build_failed

echo [2/6] Typechecking both frontends...
set "STEP=npm run typecheck"
call npm run typecheck
if errorlevel 1 goto build_failed

echo [3/6] Building the Windows CLI and worker with Cargo.lock...
set "STEP=cargo build --release -p steam-patch-cli --locked"
call cargo build --release --locked -p steam-patch-cli -p steam-patch-installer-worker --bin steam-patch --bin sf-worker
if errorlevel 1 goto build_failed

echo [4/6] Checking and building the Flutter application...
set "STEP=Flutter release build"
call pwsh -NoProfile -File scripts/build-windows-flutter.ps1 -SkipRust
if errorlevel 1 goto build_failed

echo [5/6] Generating the dependency SBOM...
if not exist ".artifacts" mkdir ".artifacts"
if errorlevel 1 (
    set "STEP=creating .artifacts"
    goto build_failed
)
set "STEP=cargo metadata --locked"
call cargo metadata --locked --format-version 1 > ".artifacts\cargo-metadata.json"
if errorlevel 1 goto build_failed
set "STEP=scripts/build-sbom.py"
call %PYTHON% "scripts\build-sbom.py" ".artifacts\cargo-metadata.json" --flutter-deps ".artifacts\windows-flutter\pub-deps.json" --flutter-version ".artifacts\windows-flutter\flutter-version.json"
if errorlevel 1 goto build_failed

echo [6/6] Packaging the portable Windows ZIP and checksums...
set "STEP=scripts/package-artifacts.py --platform windows"
call %PYTHON% "scripts\package-artifacts.py" --platform windows
if errorlevel 1 goto build_failed

set "STEP=NSIS installer"
call pwsh -NoProfile -File scripts/build-windows-installer.ps1
if errorlevel 1 goto build_failed

echo.
echo Build complete. Release files are in .artifacts.
set "RESULT=0"
goto finish

:missing_node
echo ERROR: Node.js is required. Install Node.js and make node available on PATH.
set "RESULT=1"
goto finish
:missing_npm
echo ERROR: npm is required. Install npm with Node.js and make npm available on PATH.
set "RESULT=1"
goto finish
:missing_rust
echo ERROR: Rust cargo and rustc are required. Install the Rust 1.94 MSVC toolchain,
echo or set up this repository's isolated .tools\cargo and .tools\rustup directories.
set "RESULT=1"
goto finish
:build_failed
set "RESULT=%errorlevel%"
echo.
echo ERROR: Build failed during %STEP% ^(exit code %RESULT%^).
goto finish

:finish
if defined PUSHED popd
if "%PAUSE_AT_END%"=="1" pause
exit /b %RESULT%
