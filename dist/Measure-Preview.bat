@echo off
rem Render Mill live-preview measurement (about 3 minutes).
rem Close Render Mill first so nothing else is using the graphics card.
cd /d "%~dp0"
echo Measuring live-preview refresh times on this graphics card...
echo Please leave the computer alone until this window says Done (about 3 minutes).
if exist preview-benchmark.txt del preview-benchmark.txt
start "" /wait RenderMill.exe --bench-preview small-exterior --size 1600x900 --seconds 20
start "" /wait RenderMill.exe --bench-preview large-model --size 1600x900 --seconds 20
start "" /wait RenderMill.exe --bench-preview small-exterior --size 3840x2160 --seconds 20
echo.
echo Done. Results saved to preview-benchmark.txt
pause
