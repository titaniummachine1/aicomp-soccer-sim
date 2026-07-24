@echo off
setlocal EnableExtensions
cd /d "%~dp0.."

rem Max parallel rustc + HIGH process priority for this cargo tree.
if not defined CARGO_BUILD_JOBS set CARGO_BUILD_JOBS=%NUMBER_OF_PROCESSORS%
if "%CARGO_BUILD_JOBS%"=="" set CARGO_BUILD_JOBS=8

echo === cargo_hot: -j %CARGO_BUILD_JOBS%  priority=HIGH ===
echo %*

start "cargo_hot" /HIGH /WAIT cargo -j %CARGO_BUILD_JOBS% %*
exit /b %ERRORLEVEL%
