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
# This program authors its own bitmap, then scales and blends an overlapping
# region of G0. Verify both the program's pixel reads and the composed LCD.
fixture = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-arc-blit-controls.json').read_text())
control, = (case for case in fixture['cases'] if case['name'] == 'same_grob_color_projection')
outcome = snapshots['blit_alias']['native_session']['last_outcome']
assert outcome['status'] == 'success', outcome
assert [int(item['value']) for item in outcome['typed_result']['value']] == control['expected']
frame = Image.open(out / 'native_raster_blit_alias.png').convert('RGB')
assert frame.size == (320, 240)
for index, color in enumerate(control['expected']):
    x, y = index % 5, index // 5
    expected = (color >> 16, (color >> 8) & 255, color & 255)
    assert frame.getpixel((x, y)) == expected, (x, y, frame.getpixel((x, y)), expected)
crop, = (case for case in fixture['cases'] if case['name'] == 'empty_subgraphics')
outcome = snapshots['subgrob_empty']['native_session']['last_outcome']
assert outcome['status'] == 'success', outcome
actual = [[int(item['value']) for item in row['value']] for row in outcome['typed_result']['value']]
assert actual == crop['expected'], actual
dimensions = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-grob-dimension-controls.json').read_text())
units, = (case for case in dimensions['cases'] if case['name'] == 'dimension_both_directions')
outcome = snapshots['grob_dimensions']['native_session']['last_outcome']
assert outcome['status'] == 'success', outcome
assert [float(item['value']) for item in outcome['typed_result']['value']] == units['expected']
mesh = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-mesh-point-controls.json').read_text())
packed, = (case for case in mesh['cases'] if case['name'] == 'encoded_packed_gui')
outcome = snapshots['mesh_packed_points']['native_session']['last_outcome']
assert outcome['status'] == 'success', outcome
accepted, kind, pixels, depth = outcome['typed_result']['value']
assert [int(accepted['value']), int(kind['value']), [int(p['value']) for p in pixels['value']],
        [int(word['value']) for word in depth['value']]] == packed['expected']
with Image.open(out / 'native_raster_mesh_packed_points.png') as image:
    frame = image.convert('RGB')
assert frame.size == (320, 240)
for index, color in enumerate(packed['expected'][2]):
    point = (index % 9, index // 9)
    rgb = (color >> 16, (color >> 8) & 255, color & 255)
    assert frame.getpixel(point) == rgb, (point, frame.getpixel(point), rgb)
depth_fixture = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-line-depth-controls.json').read_text())
depth_case, = (case for case in depth_fixture['cases'] if case['name'] == 'desktop_witness')
outcome = snapshots['line_depth']['native_session']['last_outcome']
assert outcome['status'] == 'success', outcome
accepted, kind, pixels, depth, initial_depth = outcome['typed_result']['value']
assert [int(accepted['value']), int(kind['value']), [int(p['value']) for p in pixels['value']],
        [int(word['value']) for word in depth['value']],
        [int(word['value']) for word in initial_depth['value']]] == depth_case['expected']
with Image.open(out / 'native_raster_line_depth.png') as image:
    frame = image.convert('RGB')
assert frame.size == (320, 240)
for index, color in enumerate(depth_case['expected'][2]):
    point = (index % 9, index // 9)
    rgb = (color >> 16, (color >> 8) & 255, color & 255)
    assert frame.getpixel(point) == rgb, (point, frame.getpixel(point), rgb)
normalization_fixture = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-line-normalization-controls.json').read_text())
normalized, = (case for case in normalization_fixture['cases'] if case['name'] == 'hold_desktop')
outcome = snapshots['line_normalized_depth']['native_session']['last_outcome']
assert outcome['status'] == 'success', outcome
accepted, kind, pixels, depth = outcome['typed_result']['value']
assert [int(accepted['value']), int(kind['value']), [int(p['value']) for p in pixels['value']],
        [int(word['value']) for word in depth['value']]] == normalized['expected']
with Image.open(out / 'native_raster_line_normalized_depth.png') as image:
    frame = image.convert('RGB')
assert frame.size == (320, 240)
for index, color in enumerate(normalized['expected'][2]):
    point = (index % 9, index // 9)
    rgb = (color >> 16, (color >> 8) & 255, color & 255)
    assert frame.getpixel(point) == rgb, (point, frame.getpixel(point), rgb)
print('Native raster: original raster pixels, overlapping BLIT, GROB dimensions, mesh points, depth and fraction results passed through IDE Run')
