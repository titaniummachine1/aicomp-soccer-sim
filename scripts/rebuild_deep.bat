@echo off
setlocal
cd /d "%~dp0.."
echo === Layer 4 DEEP: cargo clean + HIGH-priority build --release ===
echo Last resort. Multi-minute on 4c/8t = Bevy/wgpu (NN train feature is OFF by default).
cargo clean
if errorlevel 1 exit /b 1
call "%~dp0cargo_hot.bat" build --release
if errorlevel 1 exit /b 1
echo DEEP rebuild finished.
