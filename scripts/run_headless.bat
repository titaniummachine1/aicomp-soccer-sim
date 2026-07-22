@echo off
setlocal
cd /d "%~dp0.."
REM Headless match — JSON on stdout. Override args after -- 
REM Example: run_headless.bat --secs 40 --home test1 --away test2
echo Building + running soccer_headless...
cargo run --release --bin soccer_headless -- %*
