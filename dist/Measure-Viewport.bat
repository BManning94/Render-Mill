@echo off
rem Render Mill viewport smoothness measurement (about 2 minutes).
rem Close Render Mill first so nothing else is using the graphics card.
cd /d "%~dp0"
echo Measuring viewport frame times on this graphics card...
echo Please leave the computer alone until this window says Done.
if exist viewport-benchmark.txt del viewport-benchmark.txt
for %%S in (--demo --template:meadow+demo --template:valley+demo --template:lakeside+demo --template:forest+demo --template:winter+demo) do (
  start "" /wait RenderMill.exe --bench %%S --size 2560x1380 --frames 240 --quality 4
)
start "" /wait RenderMill.exe --bench --template:forest+demo --size 2560x1380 --frames 240 --quality 3
echo.
echo Done. Results saved to viewport-benchmark.txt
pause
