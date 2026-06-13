@echo off
setlocal
set SRC=%~dp0
set DST=%TEMP%\aiterm-win-test
set LOG=%SRC%win-test2-result.txt
mkdir "%DST%" 2>nul
copy /y "%SRC%aitermd.exe" "%DST%" >nul
copy /y "%SRC%aiterm.exe" "%DST%" >nul
cd /d "%DST%"
echo === aiterm Windows E2E round2 %DATE% %TIME% === > "%LOG%"
taskkill /im aitermd.exe /f >nul 2>&1
start /b "" "%DST%\aitermd.exe"
timeout /t 3 /nobreak >nul
echo --- new-surface (wait 15s for powershell cold start) --- >> "%LOG%"
aiterm.exe new-surface --title slow-start >> "%LOG%" 2>&1
timeout /t 15 /nobreak >nul
echo --- screen after 15s --- >> "%LOG%"
aiterm.exe read-screen --surface surface:1 >> "%LOG%" 2>&1
echo --- send echo --- >> "%LOG%"
aiterm.exe send --surface surface:1 "echo HELLO_WIN_ROUND2" >> "%LOG%" 2>&1
aiterm.exe send-key --surface surface:1 Return >> "%LOG%" 2>&1
timeout /t 6 /nobreak >nul
echo --- screen after echo --- >> "%LOG%"
aiterm.exe read-screen --surface surface:1 >> "%LOG%" 2>&1
echo --- scrollback lines 30 --- >> "%LOG%"
aiterm.exe read-screen --surface surface:1 --lines 30 >> "%LOG%" 2>&1
aiterm.exe close-surface surface:1 >> "%LOG%" 2>&1
taskkill /im aitermd.exe /f >nul 2>&1
echo === DONE === >> "%LOG%"
exit
