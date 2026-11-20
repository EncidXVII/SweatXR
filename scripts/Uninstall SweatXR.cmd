@echo off
rem Takes SweatXR off SteamVR's list and deletes its data. Exports in Documents\SweatXR stay.
echo This takes SweatXR off SteamVR's list and deletes its settings and workout history.
echo Exported files in Documents\SweatXR stay. Quit SweatXR and SteamVR first.
echo.
choice /M "Uninstall SweatXR"
if errorlevel 2 exit /b
"%~dp0sweatxr.exe" --uninstall
set result=%errorlevel%
echo.
type "%TEMP%\sweatxr-uninstall.txt"
if "%result%"=="0" echo Delete this folder to finish.
pause
