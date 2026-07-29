@echo off
cd /d "%~dp0"
set EXE=target\release\soccer_sim.exe
if exist "%EXE%" (
    echo Starting AIComp Soccer Sim (viewer)...
    "%EXE%"
) else (
    echo Binary not found, building release...
    cargo run --release
)
pause
