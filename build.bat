@echo off
rem Builds Solidfin and saves the compiler output to build-log.txt.
cd /d "%~dp0"
echo.
echo Building Solidfin (the first build downloads libraries and can take 5-10 minutes)...
echo.
cargo build --release --color never > build-log.txt 2>&1
if errorlevel 1 (
    echo BUILD FAILED. The errors are saved in build-log.txt - tell Claude it's done.
    echo.
    pause
    exit /b 1
)
echo Build succeeded: target\release\solidfin.exe
echo Starting Solidfin...
echo.
target\release\solidfin.exe
pause
