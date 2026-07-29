@echo off
cd /d "%~dp0"
echo Starting AIComp Soccer Sim (viewer)...
cargo run --release
pause
