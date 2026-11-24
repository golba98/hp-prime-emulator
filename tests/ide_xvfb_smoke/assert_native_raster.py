#!/usr/bin/env python3
"""Actual IDE Run and composed LCD output against complete G1 pixel controls."""
import json
import sys
from pathlib import Path
from PIL import Image

out = Path(sys.argv[1] if len(sys.argv) > 1 else '/tmp/ppl-ide-e2e')
dump = json.loads((out / 'native_side_raster.json').read_text())
assert not dump['wait_timeouts'], dump['wait_timeouts']
controls = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-ellipse-line-controls.json').read_text())
cases = {case['name']: case for case in controls['cases']}
snapshots = {snap['label']: snap['state'] for snap in dump['snapshots']}
for name, case, index in [('sector', 'ellipse-alpha-extra', 8), ('circle', 'ellipse-alpha-extra', 0), ('clipped', 'clip-extra-0', 0)]:
    outcome = snapshots[name]['native_session']['last_outcome']
    assert outcome['status'] == 'success', (name, outcome)
    assert outcome['result'].replace(' ', '') == '{1,1}', (name, outcome)
    frame = Image.open(out / f'native_raster_{name}.png').convert('RGB')
    assert frame.size == (320, 240), (name, frame.size)
    expected = cases[case]['expected'][index]
    for y in range(7):
        for x in range(7):
            color = (0 if expected[y] & (1 << x) else 0xffffff) if name == 'clipped' else expected[y * 7 + x]
            rgb = (color >> 16, (color >> 8) & 255, color & 255)
            assert frame.getpixel((x, y)) == rgb, (name, x, y, frame.getpixel((x, y)), rgb)
print('Native raster: real IDE Run, original sector/circle/clipped-line pixels and selected-version fraction program results passed')
