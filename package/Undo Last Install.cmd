@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0Install-Edits.ps1" undo
pause
