@echo off
setlocal
set SRC=%~dp0
set DST=%TEMP%\aiterm-win-test
set LOG=%SRC%win-test3-result.txt
mkdir "%DST%" 2>nul
copy /y "%SRC%aitermd.exe" "%DST%" >nul
copy /y "%SRC%aiterm.exe" "%DST%" >nul
cd /d "%DST%"
echo === round3: AITERM_SHELL=cmd.exe %DATE% %TIME% === > "%LOG%"
taskkill /im aitermd.exe /f >nul 2>&1
set AITERM_SHELL=cmd.exe
start /b "" "%DST%\aitermd.exe"
timeout /t 3 /nobreak >nul
aiterm.exe new-surface --title cmd-shell >> "%LOG%" 2>&1
timeout /t 8 /nobreak >nul
echo --- cmd screen after 8s --- >> "%LOG%"
aiterm.exe read-screen --surface surface:1 >> "%LOG%" 2>&1
aiterm.exe send --surface surface:1 "echo CMD_SHELL_WORKS" >> "%LOG%" 2>&1
aiterm.exe send-key --surface surface:1 Return >> "%LOG%" 2>&1
timeout /t 4 /nobreak >nul
echo --- cmd screen after echo --- >> "%LOG%"
aiterm.exe read-screen --surface surface:1 >> "%LOG%" 2>&1
echo --- surface list --- >> "%LOG%"
aiterm.exe list >> "%LOG%" 2>&1
aiterm.exe close-surface surface:1 >> "%LOG%" 2>&1
taskkill /im aitermd.exe /f >nul 2>&1
echo === DONE === >> "%LOG%"
exit
