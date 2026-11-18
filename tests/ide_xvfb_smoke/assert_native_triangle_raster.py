#!/usr/bin/env python3
"""Compare Native IDE triangle draw commands with original G1 pixels."""
import json
import sys
from pathlib import Path
from PIL import Image

out = Path(sys.argv[1] if len(sys.argv) > 1 else '/tmp/ppl-ide-e2e')
report = json.loads((out / 'native_side_triangle_raster.json').read_text())
assert not report['wait_timeouts'], report['wait_timeouts']
cases = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-triangle-raster-controls.json').read_text())
for label, name, width in [
    ('triangle_raster', 'original-final_desktop_rgb', 7),
    ('tall_triangle_raster', 'desktop_tall_clipped', 7),
    ('wrapped_triangle_raster', 'desktop_wrapped_crossings', 7),
    ('row_frame_word', 'row_frame_word', 9),
    ('row_frame_alpha', 'row_frame_alpha', 9),
]:
    snapshot, = (entry for entry in report['snapshots'] if entry['label'] == label)
    outcome = snapshot['state']['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    control, = (case for case in cases['cases'] if case['name'] == name)
    returned = outcome['typed_result']['value']
    assert int(returned[0]['value']) == control['expected'][0], outcome
    pixels = control['expected'][1]
    assert [int(value['value']) for value in returned[1]['value']] == pixels, name
    with Image.open(out / f'native_{label}.png') as image:
        frame = image.convert('RGB')
    assert frame.size == (320, 240), frame.size
    for index, color in enumerate(pixels):
        point = (index % width, index // width)
        expected = ((color >> 16) & 255, (color >> 8) & 255, color & 255)
        actual = frame.getpixel(point)
        assert actual == expected, (name, point, actual, expected)
print('Native triangle raster: five IDE returns and all 237 LCD pixels match the original virtual G1')
