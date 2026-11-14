@echo off
rem Takes FitnessVR off SteamVR's list and deletes its data. Exports in Documents\FitnessVR stay.
echo This takes FitnessVR off SteamVR's list and deletes its settings and workout history.
echo Exported files in Documents\FitnessVR stay. Quit FitnessVR and SteamVR first.
echo.
choice /M "Uninstall FitnessVR"
if errorlevel 2 exit /b
"%~dp0fitvr.exe" --uninstall
set result=%errorlevel%
echo.
type "%TEMP%\fitvr-uninstall.txt"
if "%result%"=="0" echo Delete this folder to finish.
pause
