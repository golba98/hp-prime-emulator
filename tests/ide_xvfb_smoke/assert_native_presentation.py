#!/usr/bin/env python3
"""Actual GUI settings, persistent entry/display modes and raw Show raster checks."""
import json
import sys
from pathlib import Path
from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
def read(name):
    data = json.loads((root / name).read_text())
    assert not data['wait_timeouts'], data['wait_timeouts']
    return {s['label']: s['state']['native_session'] for s in data['snapshots']}
n = read('native_side_presentation.json')
assert n['textbook_draft']['presentation'] == {'entry': 'textbook', 'textbook_display': True}
assert n['entry_choices']['home_input'] == '(1)/(2)'
assert n['algebraic_draft']['presentation'] == {'entry': 'algebraic', 'textbook_display': True}
assert n['algebraic_draft']['home_input'] == '(1)/(2)'
assert n['algebraic_draft']['cursor'] == n['textbook_draft']['cursor']
assert n['fraction_result']['history'][-1]['result'] == '0.5'
assert n['plain_power']['home_input'] == '2^3+1/2'
assert n['power_result']['history'][-1]['result'] == '8.5'
assert n['linear_history']['presentation'] == {'entry': 'algebraic', 'textbook_display': False}
assert n['linear_history']['history'] == n['matrix_result']['history']
assert n['matrix_show']['ans'] == n['matrix_result']['ans']
assert n['matrix_scrolled']['history_viewer']['y'] > 0
assert n['algebraic_empty_call']['home_input'] == 'SIN()'
assert n['algebraic_atomic_delete']['home_input'] == ''
assert n['algebraic_undo']['home_input'] == 'SIN()'
assert n['cas_result']['history'][-1]['typed_result'] == {'type': 'symbolic', 'value': '(x^2+1)/(2*x)'}
assert n['home_restored']['home_input'] == '7+'
assert n['home_restored']['presentation'] == {'entry': 'algebraic', 'textbook_display': False}
for name in ['matrix_show', 'matrix_scrolled']:
    image = Image.open(root / ('native_presentation_' + name + '.png')).convert('RGB')
    assert image.size == (320, 240), image.size
    for y in range(30, 220):
        assert image.getpixel((10, y)) == (17, 17, 17), (name, y, image.getpixel((10, y)))
r = read('native_side_presentation_reopen.json')
assert r['restored']['presentation'] == n['home_restored']['presentation']
assert r['restored']['home_input'] == '7+'
assert r['restored_cas']['presentation'] == n['home_restored']['presentation']
assert r['restored_cas']['history'][-1] == n['cas_result']['history'][-1]
assert r['restored_show']['history_viewer'] == {'x': 0, 'y': 0}
assert r['restored_home']['home_input'] == '7+'
print('Native presentation: independent settings, calculation, atomic deletion, persistence and matrix bracket pixels passed')
