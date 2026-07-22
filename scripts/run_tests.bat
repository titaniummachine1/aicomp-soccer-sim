@echo off
setlocal
cd /d "%~dp0.."
echo cargo test --lib
cargo test --lib
