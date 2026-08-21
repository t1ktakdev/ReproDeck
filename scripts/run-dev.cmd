@echo off
setlocal
cd /d "%~dp0.."
where node >nul 2>nul || (echo Node.js was not found on PATH.& exit /b 1)
where cargo >nul 2>nul || (echo Rust/Cargo was not found on PATH.& exit /b 1)
if not exist node_modules call npm install --no-audit || exit /b %errorlevel%
call npm run tauri dev
