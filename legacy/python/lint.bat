@echo off
:: Lint a single .hpprgm file from ANY folder — add 0-App to PATH to use globally
:: Usage: lint BSTVisualizer.hpprgm
:: Usage: lint BSTVisualizer.hpprgm --errors-only
py "%~dp0src\ppl_emulator\linter.py" %*
