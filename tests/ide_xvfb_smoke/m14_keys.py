#!/usr/bin/env python3
"""Keypad sequences (HP physical key numbers 0-50, as the IDE keypad
sends them) for text typed into the firmware's CAS view. Letters use the
key whose Alpha label is the letter; in the CAS view Alpha alone types the
lowercase letter (observed: `Alpha 7` → q, `Alpha Shift 7` → Q). Templates
work as on the keypad: `(` opens the ( ) template and `)` leaves it with
Right; `/` opens a fraction and `>` leaves a template (Right); `²` is the x²
key; `=` is Shift + `.`."""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = (ROOT / "crates/ppl-gui/src/keymap.rs").read_text()
ALPHA_KEYS = {}
for m in re.finditer(r'physical_id: (\d+),\s*ppl_code: -?\d+,\s*device_code: 0x[0-9A-Fa-f]+,\s*'
                     r'primary_label: "[^"]*",\s*secondary_label: "[^"]*",\s*alpha_label: "([^"]*)"', SRC):
    if len(m.group(2)) == 1 and m.group(2).isalpha():
        ALPHA_KEYS[m.group(2)] = int(m.group(1))
ALPHA, SHIFT, RIGHT, ENTER, ESC = 36, 41, 8, 30, 4
DIGITS = {"0": 47, "1": 42, "2": 43, "3": 44, "4": 37, "5": 38, "6": 39, "7": 32, "8": 33, "9": 34}
SYMBOLS = {"+": [50], "-": [45], "*": [40], "/": [35], ",": [29], "(": [28], ")": [RIGHT], ">": [RIGHT],
           "²": [26], "=": [SHIFT, 48], ".": [48], "π": [SHIFT, 44], "x": [17]}


def cas_keys(text):
    keys = []
    for ch in text:
        if ch in SYMBOLS:
            keys += SYMBOLS[ch]
        elif ch in DIGITS:
            keys.append(DIGITS[ch])
        elif ch.isalpha():
            keys += [ALPHA, ALPHA_KEYS[ch.upper()]]
        else:
            raise ValueError(f"no key for {ch!r}")
    return keys


if __name__ == "__main__":
    print(ALPHA_KEYS)
    print(cas_keys("int(2*x,x)"))
