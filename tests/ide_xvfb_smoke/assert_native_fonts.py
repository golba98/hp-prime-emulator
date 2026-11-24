#!/usr/bin/env python3
"""Installed font selection, independent IDE families and actual GUI/VM raster use.

Usage: assert_native_fonts.py PRIME_RUN [BUNDLED_RUN [INVALID_RUN]]
"""
import json
import sys
import hashlib
from pathlib import Path
from PIL import Image, ImageChops

def read(path):
    d = json.loads((Path(path) / 'native_side_fonts.json').read_text())
    assert not d['wait_timeouts'], d['wait_timeouts']
    return {x['label']: x['state'] for x in d['snapshots']}

prime = Path(sys.argv[1])
s = read(prime)
for label, state in s.items():
    font = state['calculator_font']
    assert font['family'] == 'Prime Sans' and font['source'] and font['error'] is None, (label, font)
    assert Image.open(prime / ('native_font_' + label.removeprefix('font_') + '.png')).size == (320, 240)
assert s['font_result']['native_session']['history'][-1]['result'] == '5'
assert s['font_program']['native_session']['last_outcome']['result'] == '17'
assert s['font_program']['native_session']['last_outcome']['status'] == 'success'
assert s['font_settings']['native_tools']['page'] == 'Settings'
# Compare actual composed tool pixels at the LCD's native resolution. The
# default smaller calculator view necessarily resamples glyphs during capture.
rect = s['font_settings']['lcd_rect']
assert [rect['max'][i] - rect['min'][i] for i in range(2)] == [320, 240]
assert all(float(n).is_integer() for n in rect['min'])

controls = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-ui-font-controls.json').read_text())
for control in controls['controls']:
    kind = control['kind']
    if kind == 'softkeys':
        # These are component-layout controls with supplied labels. The
        # installed-font native_fonts component test renders and checks their
        # golden hashes; a Home screenshot does not contain these app bars.
        # This does not establish that actual app pages bind the same labels.
        continue
    if kind == 'title':
        images = {'Function': 'native_font_home.png', 'Home Settings': 'native_font_settings.png'}
        assert control['title'] in images, ('Unknown title control', control)
        filename = images[control['title']]
    elif kind in ['expression', 'result']:
        filename = 'native_font_result.png'
    else:
        raise AssertionError(('Unknown font control kind', control))
    image = Image.open(prime / filename).convert('RGB')
    x, y, width, height = control['region']
    actual = hashlib.sha256(image.crop((x, y, x + width, y + height)).tobytes()).hexdigest()
    assert actual == control['rgb24_sha256'], (control['id'], actual)

if len(sys.argv) > 2:
    bundled = Path(sys.argv[2])
    for state in read(bundled).values():
        assert state['calculator_font']['family'] == 'DejaVu Sans'
        assert state['calculator_font']['source'] is None and state['calculator_font']['error'] is None
    for image, region in [('home', (100, 0, 220, 18)), ('palette', (45, 65, 275, 176)), ('program', (5, 15, 70, 45))]:
        a = Image.open(prime / f'native_font_{image}.png').convert('RGB').crop(region)
        b = Image.open(bundled / f'native_font_{image}.png').convert('RGB').crop(region)
        assert ImageChops.difference(a, b).getbbox(), f'{image}: GUI still used the bundled raster'
if len(sys.argv) > 3:
    invalid = Path(sys.argv[3])
    for label, state in read(invalid).items():
        font = state['calculator_font']
        assert font['family'] == 'unavailable' and font['source'] and font['error'], (label, font)
    error_lcd = Image.open(invalid / 'native_font_home.png').convert('RGB')
    assert error_lcd.getpixel((0, 0)) == (255, 255, 255)
    assert any(max(error_lcd.getpixel((x, y))) < 150 for y in range(40, 100) for x in range(8, 300))
print('Installed Prime GUI fonts: four independent G1 pixel regions, actual results/rasters and explicit resource errors passed')
