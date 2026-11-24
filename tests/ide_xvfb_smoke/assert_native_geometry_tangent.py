"""Geometry Tangent through Native application input, PPL, and restart."""
import json
import math
from pathlib import Path
import sys

from PIL import Image

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def snapshots(suffix=''):
    name = f'native_side_geometry_tangent{suffix}.json'
    report = json.loads((ROOT / name).read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {entry['label']: entry['state'] for entry in report['snapshots']}


def book(state):
    return state['native_tools']['memory']['models']['geometry_book']


def objects(state):
    scene = state['native_model_results']['Geometry:plot']['geometry']
    return {item['name']: item['value']['Ok'] for item in scene['objects']}


def outcome(state):
    result = state['native_session']['last_outcome']
    assert result['status'] == 'success', result
    assert result['typed_result']['type'] == 'list', result
    return result['typed_result']['value']


def number(item):
    assert item['type'] == 'real', item
    return float(item['value'])


def symbolic(item):
    assert item['type'] == 'symbolic', item
    return item['value']['value']


def plotted_pixel(label, state, x, y, threshold=110):
    window = state['native_tools']['memory']['model_windows']['Geometry']
    px = 319 * (x - window['xmin']) / (window['xmax'] - window['xmin'])
    py = 219 - 219 * (y - window['ymin']) / (window['ymax'] - window['ymin'])
    left, top = state['lcd_rect']['min']
    right, bottom = state['lcd_rect']['max']
    sx = round(left + px * (right - left) / 320)
    sy = round(top + py * (bottom - top) / 240)
    with Image.open(ROOT / f'geometry_tangent_{label}.png').convert('RGB') as image:
        neighborhood = image.crop((sx - 1, sy - 1, sx + 2, sy + 2))
        return any(
            max(neighborhood.getpixel((px, py))) < threshold
            for px in range(3) for py in range(3)
        )


states = snapshots()
restored = snapshots('_reopen')['reopened']
prepared = states['prepared']
rejected = states['free_point_rejected']
created = states['canvas_created']
full_keypad = states['full_keypad']
redrawn = states['redrawn']

assert [number(item) for item in outcome(prepared)] == [6, 2, 6, 0, 8, 0, 2]
assert rejected['native_tools']['error'] == 'Select a point on a curve'
assert book(rejected) == book(prepared)
for label, state in [('canvas_created', created), ('redrawn', redrawn), ('reopened', restored)]:
    tools = state['native_tools']
    assert tools['page'] == 'Plot' and not tools['calculating'] and tools['error'] is None, tools
    assert 'current_values' not in book(state)
    assert 'current_point_curves' not in book(state)
    assert [(item['name'], item['source']) for item in book(state)['objects'][-3:]] == [
        ('E', 'tangent(GB,GD)'), ('F', 'tangent(GB,point(1,0))'), ('G', 'tangent(GC)'),
    ]
    curves = objects(state)
    assert curves['F']['Lines'] == []
    for (anchor, contact), sign in zip(curves['E']['Lines'], (-1, 1), strict=True):
        assert anchor == {'x': 4., 'y': 0.}
        assert math.isclose(contact['x'], 1.)
        assert math.isclose(contact['y'], sign * math.sqrt(3.))
        assert plotted_pixel(label, state, 7., sign * math.sqrt(3.)), (label, sign)

assert not plotted_pixel('prepared', prepared, 9., 2.)
assert plotted_pixel('canvas_created', created, 9., 2.)
assert full_keypad['calculator_mode'] == 'Full'
assert book(full_keypad) == book(created)
# The line passes well outside the circle, axes and exterior tangents.
# Dimming during minification is visible, while the old sampler shows only grid or white.
assert plotted_pixel('full_keypad', full_keypad, 9., 2., threshold=160)
with (
    Image.open(ROOT / 'geometry_tangent_raw_before.png') as first,
    Image.open(ROOT / 'geometry_tangent_raw_after.png') as returned,
    Image.open(ROOT / 'geometry_tangent_native_size.png') as native,
):
    assert first.size == returned.size == native.size == (320, 240)
    assert first.convert('RGB').tobytes() == returned.convert('RGB').tobytes()
    assert first.convert('RGB').getpixel((250, 89)) == (0, 0, 0)
    # Check the stroke and its adjacent pixels at one physical pixel per texel.
    stroke = (224, 78, 277, 106)
    assert first.convert('RGB').crop(stroke).tobytes() == native.convert('RGB').crop(stroke).tobytes()
horizontal = objects(created)['G']['Line']
assert horizontal['extent'] == 'Line' and math.isclose(horizontal['a']['y'], 2.)
assert math.isclose(horizontal['b']['y'], 2.)
for label, state in [('redrawn', redrawn), ('reopened', restored)]:
    assert objects(state)['G']['Line'] == {
        'a': {'x': 2., 'y': 0.}, 'b': {'x': 2., 'y': 2.}, 'extent': 'Line',
    }
    assert plotted_pixel(label, state, 2., 6.), label
    assert not plotted_pixel(label, state, 9., 2.), label

values = outcome(redrawn)
assert abs(number(values[0])) < 1e-10 and number(values[0]) == number(values[1])
expected = ['line(point(2,0),point(2,2))', 'line(point(2,0),point(2,2))', 'tangent(GC)']
assert [symbolic(item) for item in values[2:]] == expected
values = outcome(restored)
assert [symbolic(item) for item in values[:3]] == expected
assert [number(item) for item in values[3:]] == [2, 0]
assert book(restored) == book(redrawn)
print('Geometry Tangent passed for typed PPL, construction, minified display, raw capture and restart')
