@echo off
setlocal
cd /d "%~dp0.."
echo Building + launching viewer (soccer_sim)...
cargo run --release
