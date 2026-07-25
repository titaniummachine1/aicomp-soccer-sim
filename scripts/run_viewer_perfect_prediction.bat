@echo off
REM Visible PerfectController prediction (purple path + terminal disc).
REM P1 = WASD + E kick. Toggle Debug checkbox top-right if lines missing.
REM Away team parked idle.
cd /d "%~dp0.."
set GRAPH=%USERPROFILE%\AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\PerfectController.txt
if not exist "%GRAPH%" (
  echo Missing %GRAPH% — run worldcupteams\probes\build_perfect_controller.py first
  exit /b 1
)
echo Launching viewer: PerfectController vs idle (fast)...
cargo run -- --fast --home "graph:%GRAPH%" --away idle --opening home
