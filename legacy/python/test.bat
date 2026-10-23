@echo off
:: Run all .hpprgm tests from ANY folder — add 0-App to PATH to use this globally
:: Usage: test
:: Usage: test -v
cd /d "%~dp0" && py tests\test_integration.py %*
