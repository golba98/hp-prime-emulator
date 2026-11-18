"""Verify the Native Ellipse menu, constrained points, measurements and restart."""
import json
import math
from pathlib import Path
import sys

from PIL import Image


ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
DEFINITION = 'ellipse(GA,GB,GC)'


def snapshots(suffix=''):
    file = ROOT / f'native_side_geometry_ellipse{suffix}.json'
    report = json.loads(file.read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {item['label']: item['state'] for item in report['snapshots']}


def book(state):
    return state['native_tools']['memory']['models']['geometry_book']


def geometry(state):
    return state['native_model_results']['Geometry:plot']['geometry']


def shape(state, name, kind):
    return next(obj['value']['Ok'][kind] for obj in geometry(state)['objects']
                if obj['name'] == name)


def close(value, expected, tolerance=1e-8):
    assert math.isclose(value, expected, abs_tol=tolerance, rel_tol=0), (value, expected)


def constrained(state):
    p = shape(state, 'E', 'Point')
    ellipse = shape(state, 'D', 'Ellipse')
    x, y = p['x'] - ellipse['center']['x'], p['y'] - ellipse['center']['y']
    u, v = ellipse['axis']['x'], ellipse['axis']['y']
    close(((u*x + v*y) / ellipse['major'])**2 + ((u*y - v*x) / ellipse['minor'])**2, 1.)
    source = book(state)['objects'][4]['source']
    assert source.startswith('element(GD,') and source.endswith(')'), source
    witness = ellipse['size']['Witness']
    close(float(source[len('element(GD,'):-1]), (p['y'] - witness['y']) / (p['x'] - witness['x']))
    assert 'E' in geometry(state)['point_curves']
    return p


def ink(label, state, x, y):
    window = state['native_tools']['memory']['model_windows']['Geometry']
    px = 319 * (x - window['xmin']) / (window['xmax'] - window['xmin'])
    py = 219 - 219 * (y - window['ymin']) / (window['ymax'] - window['ymin'])
    left, top = state['lcd_rect']['min']
    right, bottom = state['lcd_rect']['max']
    sx = round(left + px * (right - left) / 320)
    sy = round(top + py * (bottom - top) / 240)
    with Image.open(ROOT / f'geometry_ellipse_{label}.png').convert('RGB') as image:
        colors = {(0, 0, 0), (255, 128, 128), (0, 191, 255)}
        return any(image.getpixel((sx + dx, sy + dy)) in colors
                   for dx in range(-2, 3) for dy in range(-2, 3))


def read_values(state):
    result = state['native_session']['last_outcome']
    assert result['status'] == 'success', result
    typed = result['typed_result']
    assert typed['type'] == 'list', typed
    values = typed['value']
    assert all(value['type'] == 'real' for value in values[:-1]), values
    assert values[-1]['type'] == 'symbolic'
    assert values[-1]['value'] == {'type': 'symbolic', 'value': DEFINITION}
    return [float(value['value']) for value in values[:-1]]


states = snapshots()
restored = snapshots('_reopen')['reopened']
for label, state in [*states.items(), ('reopened', restored)]:
    tools = state['native_tools']
    assert tools['page'] == 'Plot' and not tools['calculating'] and tools['error'] is None, label

initial = ['point(-2,0)', 'point(2,0)', 'point(0,3)']
for label in ('prepared', 'tool_open', 'focus_one', 'focus_two'):
    assert [o['source'] for o in book(states[label])['objects']] == initial
created = states['ellipse_created']
assert [o['source'] for o in book(created)['objects']] == initial + [DEFINITION]
before = (math.sqrt(13/2), -3/math.sqrt(2))
assert not ink('prepared', states['prepared'], *before)
assert ink('ellipse_created', created, *before)

placed = constrained(states['point_created'])
dragged = constrained(states['point_moved'])
assert placed['x'] > 1.5 and placed['y'] > 1.
assert dragged['x'] > 1.5 and dragged['y'] < -1.
assert math.dist(tuple(placed.values()), tuple(dragged.values())) > 3.
assert ink('point_created', states['point_created'], *placed.values())
assert ink('point_moved', states['point_moved'], *dragged.values())

for label, state in [('focus_moved', states['focus_moved']), ('reopened', restored)]:
    focus = shape(state, 'B', 'Point')
    close(focus['x'], 4., 0.12)
    close(focus['y'], 0., 0.12)
    ellipse = shape(state, 'D', 'Ellipse')
    assert ellipse['major'] > 4.2 and ellipse['minor'] > 3.
    edge = (ellipse['center']['x'] + ellipse['major'] / math.sqrt(2),
            ellipse['center']['y'] - ellipse['minor'] / math.sqrt(2))
    assert ink(label, state, *edge)
    assert not ink(label, state, *before)
    assert book(state)['objects'][3]['source'] == DEFINITION
    constrained(state)

values = read_values(states['values'])
assert values == read_values(restored)
assert values[:2] == [8., 8.] and values[10:12] == [0., 8.]
focus = shape(restored, 'B', 'Point')
p = constrained(restored)
for value, expected in zip(values[2:6], [focus['x'], focus['y'], p['x'], p['y']]):
    close(value, expected)
close(values[7], 3. + values[6] / 2.)
ellipse = shape(restored, 'D', 'Ellipse')
close(values[8], math.pi * ellipse['major'] * ellipse['minor'])
assert 23.0 < values[9] < 23.9
close(values[12], -6/11)
assert book(restored) == book(states['focus_moved'])
print('Geometry Ellipse passed for three-point input, constrained drag, focus redraw, typed PPL and restart')
