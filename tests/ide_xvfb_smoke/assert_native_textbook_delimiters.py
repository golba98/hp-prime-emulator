"""Check absolute-value composition and grouping in real CAS History and Show."""
from itertools import groupby
import json
from pathlib import Path
import sys

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
report = json.loads((root / 'native_side_textbook_delimiters.json').read_text())
assert not report['wait_timeouts'], report['wait_timeouts']
states = {item['label']: item['state']['native_session'] for item in report['snapshots']}
for label, expression in (
    ('fraction', 'abs(-3/2)'),
    ('nested', 'abs(-3/(2+sqrt(5/7)))'),
    ('factorial', '(x^2)!'),
):
    entry = states[label]['history'][-1]
    assert entry['expression'] == expression and not entry['error'], entry
assert states['fraction']['history'][-1]['typed_result'] == {
    'type': 'rational', 'value': {'numerator': '3', 'denominator': '2'},
}
assert states['nested']['history'][-1]['typed_result']['type'] == 'symbolic'
assert states['factorial']['history'][-1]['typed_result'] == {
    'type': 'symbolic', 'value': '(x^2)!',
}
assert states['show']['history_viewer'] == {'x': 0, 'y': 0}
assert states['show']['history_result'] is False
assert states['show']['history'] == states['nested']['history']


def capture(label):
    image = Image.open(root / f'textbook_delimiters_{label}.png').convert('L')
    assert image.size == (320, 240)
    return image


def runs(coordinates, dark):
    return [pixels for ink, group in groupby(coordinates, dark)
            if ink and (pixels := list(group))]


def absolute_structure(label, top, bottom, fraction_and_root_lines):
    image = capture(label)
    vertical = [(x, stroke) for x in range(105)
                for stroke in runs(range(top, bottom), lambda y: image.getpixel((x, y)) < 96)
                if len(stroke) >= 25]
    # The outer absolute-value bars also enclose the nested radical's stem.
    assert len(vertical) >= 2, (label, 'absolute-value bars', vertical)
    (left, left_stroke), (right, right_stroke) = vertical[0], vertical[-1]
    assert left_stroke == right_stroke
    assert all(left < x < right for x, _ in vertical[1:-1])
    within_bars = range(left + 1, right)
    horizontal = [(y, stroke) for y in left_stroke
                  for stroke in runs(within_bars, lambda x: image.getpixel((x, y)) < 96)
                  if len(stroke) >= 10]
    assert len(horizontal) == fraction_and_root_lines, (label, horizontal)


absolute_structure('fraction', 125, 200, 1)
absolute_structure('nested', 137, 200, 3)
absolute_structure('show', 20, 200, 3)
factorial = capture('factorial')
glyph_columns = runs(range(65), lambda x: any(factorial.getpixel((x, y)) < 96 for y in range(170, 197)))
assert len(glyph_columns) == 5, 'grouped factorial should show ( x superscript-2 ) !'
print('Native CAS History and Show enclose stacked fractions, retain grouped factorials and keep exact values')
