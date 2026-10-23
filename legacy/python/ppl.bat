@echo off
:: Run any .hpprgm file from ANY folder — add 0-App to PATH to use this globally
:: Usage: ppl BSTVisualizer.hpprgm
:: Usage: ppl BSTVisualizer.hpprgm --dump-python
:: Usage: ppl BSTVisualizer.hpprgm --output myscreen.png
py "%~dp0src\ppl_emulator\cli.py" %*
