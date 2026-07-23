@echo off
setlocal
cd /d "%~dp0.."
echo Building + launching viewer (STATIC — slow link, no Bevy DLLs)...
cargo run --release --no-default-features
