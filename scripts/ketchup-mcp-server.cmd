@echo off
rem MCP server for AI clients working on this checkout, e.g. in mcp_servers.json:
rem   "command": "C:\\Sources8\\Ketchup\\scripts\\ketchup-mcp-server.cmd"
rem The server and the windows it opens run from copies under target\mcp-*,
rem so `cargo build --release` can always replace target\release.
rem Stdout belongs to MCP: every other output goes to NUL.
setlocal
set "RELEASE=%~dp0..\target\release"
set "SERVER=%~dp0..\target\mcp-server"
if not exist "%SERVER%" mkdir "%SERVER%" >NUL 2>&1
rem A server that is still running keeps its copy; the copy is refreshed next time.
copy /Y "%RELEASE%\ketchup-app.exe" "%SERVER%\ketchup-app.exe" >NUL 2>&1
xcopy /D /Y /Q "%RELEASE%\*.dll" "%SERVER%\" >NUL 2>&1
set "KETCHUP_WINDOW_SOURCE=%RELEASE%\ketchup-app.exe"
set "KETCHUP_MCP_STAGE_DIR=%~dp0..\target\mcp-windows"
"%SERVER%\ketchup-app.exe" --mcp
