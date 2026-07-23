@echo off
setlocal
cd /d "%~dp0.."
echo === Layer 4 DEEP STATIC: wipe all + static release ===
cargo clean
if errorlevel 1 exit /b 1
cargo build --release --no-default-features --profile release-static
if errorlevel 1 exit /b 1
echo DEEP static rebuild finished.
