@echo off
setlocal EnableExtensions EnableDelayedExpansion
chcp 65001 >nul
title 瓦刻 / VALOFRAME - Dev

set "ROOT=%~dp0"
pushd "%ROOT%" || (
  echo Failed to enter project directory: %ROOT%
  echo.
  pause
  exit /b 1
)

echo Starting 瓦刻 / VALOFRAME desktop dev environment...
echo Project: %CD%
echo.

set "CARGO_INCREMENTAL=0"
echo Rust incremental compilation disabled for this dev session.
echo.

if not exist "node_modules\" (
  echo Dependencies are missing: node_modules was not found.
  echo Run "npm install" first, then start this script again.
  echo.
  pause
  popd
  exit /b 1
)

where npm >nul 2>&1
if errorlevel 1 (
  echo Could not find npm. Install Node.js, then start this script again.
  set "EXIT_CODE=1"
  goto finish
)

echo Using npm: npm run tauri -- dev
echo.
npm run tauri -- dev
set "EXIT_CODE=!ERRORLEVEL!"

:finish
echo.
if not "%EXIT_CODE%"=="0" (
  echo Dev command exited with code %EXIT_CODE%.
)
echo Press any key to close this window.
pause >nul
popd
exit /b %EXIT_CODE%
