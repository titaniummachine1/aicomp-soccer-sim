@echo off
setlocal
cd /d "%~dp0.."
REM Left=AIA + Right=AIA node inventory + cargo test + headless match.
REM Extra args forwarded to validate_aia_lr.py (e.g. --secs 40 --seed 3).
python scripts\validate_aia_lr.py %*
exit /b %ERRORLEVEL%
