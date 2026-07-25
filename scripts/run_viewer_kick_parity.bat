@echo off
REM Visible kick-routine test (Bevy viewer). P1 starts at center WITH ball in Play phase.
REM Sequence: +45 full, pickup, center, -45 full, pickup, center, -45 50%%
cd /d "%~dp0.."
echo Launching viewer: kick vs idle (fast, harnessed)...
cargo run -- --fast --home kick --away idle
