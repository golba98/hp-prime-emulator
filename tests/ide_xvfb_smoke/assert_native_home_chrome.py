#!/usr/bin/env python3
"""G1 Home geometry and actual pointer/store actions, captured from the desktop."""
import json
import sys
from pathlib import Path
from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
data = json.loads((root / 'native_side_home_chrome.json').read_text())
assert data['wait_timeouts'] == [], data['wait_timeouts']
states = {item['label']: item['state'] for item in data['snapshots']}
blank = Image.open(root / 'native_home_chrome_blank.png').convert('RGB')
assert blank.size == (320, 240)
# Dimensions and regions measured from the genuine G1 LCD. This catches the old
# 24px solid header, blue 38px entry strip, and evenly split blue softkey strip.
assert blank.getpixel((3, 0))[2] > blank.getpixel((3, 0))[0]
assert blank.getpixel((3, 0)) != blank.getpixel((3, 17))
assert blank.getpixel((3, 18)) == (0, 0, 0)
assert blank.getpixel((3, 19)) == (137, 137, 137)
assert blank.getpixel((3, 20)) == (255, 255, 255)
assert all(blank.getpixel((100, y)) == (255, 255, 255) for y in range(203, 220))
assert blank.getpixel((100, 202)) == (0, 0, 0)
for x in [0, 51, 53, 104, 106, 157, 159, 210, 212, 263, 265, 319]:
    assert blank.getpixel((x, 225)) == (0, 0, 0), x
for x in [52, 105, 158, 211, 264]:
    assert blank.getpixel((x, 225)) == (255, 255, 255), x
result = Image.open(root / 'native_home_chrome_result.png').convert('RGB')
ink_rows = [y for y in range(20, 202) if any(max(result.getpixel((x, y))) < 120 for x in range(26))]
assert (min(ink_rows), max(ink_rows)) == (189, 198)
fraction = Image.open(root / 'native_home_chrome_fraction.png').convert('RGB')
assert fraction.getpixel((100, 183)) == (0, 0, 0)
assert fraction.getpixel((100, 184)) == (255, 255, 255)
assert states['store_inserted']['native_session']['home_input'] == 'Ans▶'
assert states['stored']['calculator_memory']['A'] == {'type': 'real', 'value': '5', 'origin': {'kind': 'rational', 'numerator': '5', 'denominator': '1', 'negative': False}}
assert states['fraction']['native_session']['home_input'] == '(2)/(3)'
assert states['pointer_numerator']['native_session']['home_input'] == '(42)/(3)'
assert states['fraction_result']['native_session']['history'][-1]['result'] == '14'
assert states['app_title']['native_session']['app_title'] == 'Parametric'
assert len(states['app_title']['native_session']['status_clock']) == 5
assert all(not entry['error'] for entry in states['app_title']['native_session']['history'])
print('G1 Home: measured header/entry/softkey geometry, history baseline, pointer fraction edit and actual Sto write passed')
