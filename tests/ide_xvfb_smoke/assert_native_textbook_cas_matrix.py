"""Exact CAS values and visible fraction structure in History and Show."""
import json
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
report = json.loads((root / 'native_side_textbook_cas_matrix.json').read_text())
assert not report['wait_timeouts'], report['wait_timeouts']
snapshots = {item['label']: item['state']['native_session'] for item in report['snapshots']}
for label in ('history', 'show'):
    entry = snapshots[label]['history'][-1]
    assert not entry['error']
    typed = entry['typed_result']
    assert typed['type'] == 'matrix'
    matrix = typed['value']
    assert (matrix['rows'], matrix['cols']) == (2, 2)
    assert all(cell['type'] == 'rational' for cell in matrix['values'])
    assert [(cell['value']['numerator'], cell['value']['denominator'])
            for cell in matrix['values']] == [('1','2'), ('2','3'), ('3','4'), ('4','5')]
assert snapshots['history']['history_viewer'] is None
assert snapshots['show']['history_viewer'] == {'x': 0, 'y': 0}


def bars(image_name, bounds):
    """Find line segments longer than the digits and narrow bracket tips."""
    image = Image.open(root / f'textbook_cas_matrix_{image_name}.png').convert('L')
    assert image.size == (320, 240)
    x0, x1, y0, y1 = bounds
    widths_by_row = {}
    for y in range(y0, y1):
        dark_start = None
        for x in range(x0, x1 + 1):
            dark = x < x1 and image.getpixel((x, y)) < 96
            if dark and dark_start is None:
                dark_start = x
            if not dark and dark_start is not None:
                width = x - dark_start
                if width >= 10:
                    widths_by_row.setdefault(y, []).append(width)
                dark_start = None
    return sorted(len(widths) for widths in widths_by_row.values())


assert bars('history', (0, 84, 95, 198)) == [2, 2], 'source matrix fraction strokes'
assert bars('history', (236, 320, 95, 198)) == [2, 2], 'result matrix fraction strokes'
assert bars('show', (6, 90, 20, 200)) == [2, 2], 'Show matrix fraction strokes'
print('Native CAS matrix paints all four exact fractions in the expression, result and Show')
