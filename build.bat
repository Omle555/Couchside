@echo off
rem Checks formatting, lints and tests Couchside, then builds it.
rem All compiler output is saved to build-log.txt.
cd /d "%~dp0"
echo.
echo Checking Couchside (the first run downloads libraries and can take 5-10 minutes)...
echo.

echo [1/4] Checking formatting (cargo fmt --check)...
cargo fmt --all --check > build-log.txt 2>&1
if errorlevel 1 (
    echo FORMAT CHECK FAILED. Run "cargo fmt --all" to fix it. Details are in build-log.txt.
    echo.
    pause
    exit /b 1
)

echo [2/4] Running clippy...
cargo clippy --all-targets --all-features --color never -- -D warnings >> build-log.txt 2>&1
if errorlevel 1 (
    echo CLIPPY FAILED. The warnings are saved in build-log.txt - tell Claude it's done.
    echo.
    pause
    exit /b 1
)

echo [3/4] Running tests...
cargo test --workspace --color never >> build-log.txt 2>&1
if errorlevel 1 (
    echo TESTS FAILED. The failures are saved in build-log.txt - tell Claude it's done.
    echo.
    pause
    exit /b 1
)

echo [4/4] Building release...
cargo build --release --color never >> build-log.txt 2>&1
if errorlevel 1 (
    echo BUILD FAILED. The errors are saved in build-log.txt - tell Claude it's done.
    echo.
    pause
    exit /b 1
)
echo Build succeeded: target\release\couchside.exe
echo Starting Couchside...
echo.
target\release\couchside.exe
pause
