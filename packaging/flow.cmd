@echo off
REM SPDX-License-Identifier: Apache-2.0
setlocal
"%~dp0flow-cli.exe" %*
exit /b %errorlevel%
