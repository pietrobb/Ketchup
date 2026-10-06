@echo off
rem MCP server for AI clients working on this checkout, e.g. in mcp_servers.json:
rem   "command": "C:\\Sources8\\Ketchup\\scripts\\ketchup-mcp-server.cmd"
rem The server and the windows it opens run from copies under target\mcp-*,
rem so `cargo build --release` can always replace target\release.
rem Each build gets its own server copy named by the size and time of the exe,
rem so a new server starts the newest build while an older one still runs.
rem Stdout belongs to MCP: every other output goes to NUL.
setlocal
set "RELEASE=%~dp0..\target\release"
set "SERVERS=%~dp0..\target\mcp-server"
for %%F in ("%RELEASE%\ketchup-app.exe") do set "STAMP=%%~zF_%%~tF"
set "STAMP=%STAMP::=%"
set "STAMP=%STAMP:/=%"
set "STAMP=%STAMP:.=%"
set "STAMP=%STAMP: =_%"
set "SERVER=%SERVERS%\%STAMP%"
rem Copies of older builds go once their server has ended (its exe is no longer locked).
for /d %%D in ("%SERVERS%\*") do if /i not "%%~nxD"=="%STAMP%" (
    del /Q "%%D\ketchup-app.exe" >NUL 2>&1
    if not exist "%%D\ketchup-app.exe" rmdir /S /Q "%%D" >NUL 2>&1
)
if not exist "%SERVER%\ketchup-app.exe" (
    mkdir "%SERVER%" >NUL 2>&1
    xcopy /Y /Q "%RELEASE%\*.dll" "%SERVER%\" >NUL 2>&1
    copy /Y "%RELEASE%\ketchup-app.exe" "%SERVER%\ketchup-app.exe" >NUL 2>&1
)
set "KETCHUP_WINDOW_SOURCE=%RELEASE%\ketchup-app.exe"
set "KETCHUP_MCP_STAGE_DIR=%~dp0..\target\mcp-windows"
"%SERVER%\ketchup-app.exe" --mcp
