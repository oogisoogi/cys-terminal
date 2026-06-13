@echo off
setlocal
set SRC=%~dp0
set DST=%TEMP%\aiterm-win-test
set LOG=%SRC%win-test-result.txt
rmdir /s /q "%DST%" 2>nul
mkdir "%DST%"
copy /y "%SRC%aitermd.exe" "%DST%" >nul
copy /y "%SRC%aiterm.exe" "%DST%" >nul
cd /d "%DST%"
echo === aiterm Windows E2E %DATE% %TIME% === > "%LOG%"
ver >> "%LOG%"
taskkill /im aitermd.exe /f >nul 2>&1
start /b "" "%DST%\aitermd.exe"
timeout /t 3 /nobreak >nul
echo --- ping --- >> "%LOG%"
aiterm.exe ping >> "%LOG%" 2>&1
echo --- identify --- >> "%LOG%"
aiterm.exe identify >> "%LOG%" 2>&1
echo --- new-surface --- >> "%LOG%"
aiterm.exe new-surface --title win-test >> "%LOG%" 2>&1
timeout /t 4 /nobreak >nul
echo --- send + send-key --- >> "%LOG%"
aiterm.exe send --surface surface:1 "echo HELLO_FROM_WINDOWS_PIPE" >> "%LOG%" 2>&1
aiterm.exe send-key --surface surface:1 Return >> "%LOG%" 2>&1
timeout /t 3 /nobreak >nul
echo --- read-screen --- >> "%LOG%"
aiterm.exe read-screen --surface surface:1 >> "%LOG%" 2>&1
echo --- feed push/list --- >> "%LOG%"
aiterm.exe feed push --title "win approval" --body "windows e2e" >> "%LOG%" 2>&1
aiterm.exe feed list >> "%LOG%" 2>&1
echo --- close-surface --- >> "%LOG%"
aiterm.exe close-surface surface:1 >> "%LOG%" 2>&1
taskkill /im aitermd.exe /f >nul 2>&1
echo === DONE === >> "%LOG%"
exit
