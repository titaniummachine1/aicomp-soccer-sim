@echo off
setlocal
cd /d "%~dp0.."
echo === Layer 3: crate-only rebuild (Bevy / deps stay) ===
cargo clean -p aicomp-soccer-sim
if errorlevel 1 exit /b 1
cargo build --release
if errorlevel 1 exit /b 1
echo Crate rebuilt. If still broken, run scripts\rebuild_deep.bat
