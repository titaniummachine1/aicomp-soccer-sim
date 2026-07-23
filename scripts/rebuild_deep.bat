@echo off
setlocal
cd /d "%~dp0.."
echo === Layer 4 DEEP: cargo clean + build --release (wipes Bevy + all deps) ===
echo Last resort after rebuild_crate.bat. Can take 10+ min.
cargo clean
if errorlevel 1 exit /b 1
cargo build --release
if errorlevel 1 exit /b 1
echo DEEP rebuild finished.
