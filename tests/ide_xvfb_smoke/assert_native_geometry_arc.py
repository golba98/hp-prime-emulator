"""Verify Native circular arc entry, partial drawing, measurements and reopen."""
import json
import math
from pathlib import Path
import sys

from PIL import Image


ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
DEFINITION = 'circle(GA,GB,0,π/2)'


def snapshots(suffix=''):
    file = ROOT / f'native_side_geometry_arc{suffix}.json'
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


def ink(label, state, x, y):
    window = state['native_tools']['memory']['model_windows']['Geometry']
    px = 319 * (x - window['xmin']) / (window['xmax'] - window['xmin'])
    py = 219 - 219 * (y - window['ymin']) / (window['ymax'] - window['ymin'])
    left, top = state['lcd_rect']['min']
    right, bottom = state['lcd_rect']['max']
    sx = round(left + px * (right - left) / 320)
    sy = round(top + py * (bottom - top) / 240)
    with Image.open(ROOT / f'geometry_arc_{label}.png').convert('RGB') as image:
        colors = {(0, 0, 0), (255, 128, 128), (0, 191, 255)}
        return any(image.getpixel((sx + dx, sy + dy)) in colors
                   for dx in range(-2, 3) for dy in range(-2, 3))


def arc_plot(label, state):
    arc = shape(state, 'C', 'Arc')
    cx, cy = arc['center']['x'], arc['center']['y']
    diagonal = arc['radius'] / math.sqrt(2)
    assert ink(label, state, cx + diagonal, cy + diagonal), label
    for sign_x, sign_y in ((-1, 1), (-1, -1), (1, -1)):
        p = (cx + sign_x * diagonal, cy + sign_y * diagonal)
        assert not ink(label, state, *p), (label, p)
    close(arc['start'], 0.)
    close(arc['end'], math.pi / 2)
    return arc


def constrained(state):
    arc = shape(state, 'C', 'Arc')
    p = shape(state, 'D', 'Point')
    source = book(state)['objects'][3]['source']
    assert source.startswith('element(GC,') and source.endswith(')'), source
    angle = float(source[len('element(GC,'):-1])
    close(p['x'], arc['center']['x'] + arc['radius'] * math.cos(angle))
    close(p['y'], arc['center']['y'] + arc['radius'] * math.sin(angle))
    assert 'D' in geometry(state)['point_curves']
    return p, angle


def values(state):
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    result = outcome['typed_result']
    assert result['type'] == 'list', result
    nested = result['value']
    assert all(value['type'] == 'real' for value in nested[:-1]), nested
    assert nested[-1]['type'] == 'symbolic'
    assert nested[-1]['value']['value'] == DEFINITION
    return [float(value['value']) for value in nested[:-1]]


states = snapshots()
restored = snapshots('_reopen')['reopened']
for label, state in [*states.items(), ('reopened', restored)]:
    tools = state['native_tools']
    page = 'Symbolic' if label in ('prepared', 'symbolic') else 'Plot'
    assert tools['page'] == page and tools['error'] is None and not tools['calculating'], label

initial = ['point(-3,1)', 'point(5,1)']
assert [o['source'] for o in book(states['prepared'])['objects']] == initial
assert [o['source'] for o in book(states['symbolic'])['objects']] == initial + [DEFINITION]
arc = arc_plot('arc_created', states['arc_created'])
assert arc['center'] == {'x': 1., 'y': 1.}
close(arc['radius'], 4.)

first, first_angle = constrained(states['point_placed'])
close(first_angle, math.pi / 4, 0.05)
close(first['x'], 1. + math.sqrt(8), 0.2)
close(first['y'], 1. + math.sqrt(8), 0.2)
moved, moved_angle = constrained(states['point_dragged'])
close(moved_angle, -math.pi / 2, 0.05)
close(moved['x'], 1., 0.2)
close(moved['y'], -3., 0.2)
assert ink('point_dragged', states['point_dragged'], *moved.values())

for label, state in [('diameter_moved', states['diameter_moved']), ('reopened', restored)]:
    arc = arc_plot(label, state)
    a = shape(state, 'A', 'Point')
    close(a['x'], -5., 0.15)
    close(a['y'], 3., 0.15)
    assert arc['center'] == {'x': (a['x'] + 5.) / 2., 'y': (a['y'] + 1.) / 2.}
    close(arc['radius'], math.hypot(a['x'] - 5., a['y'] - 1.) / 2.)
    p, parameter = constrained(state)
    assert parameter == moved_angle
    assert p != moved
    assert book(state)['objects'][2]['source'] == DEFINITION

measured = values(states['values'])
assert measured == values(restored)
assert measured[:2] == [8., 8.] and measured[11] == 8.
arc = shape(restored, 'C', 'Arc')
cx, cy = arc['center']['x'], arc['center']['y']
radius = arc['radius']
p, _ = constrained(restored)
expected = [
    cx, cy, radius, math.pi*radius/2., math.pi*radius**2/4.,
    cx-radius, cy, p['x'], p['y'], 8., -3.*math.pi, -3.*math.pi, 0., 0., 4.*math.pi,
]
assert len(measured[2:]) == len(expected)
for value, reference in zip(measured[2:], expected):
    close(value, reference)
assert book(restored) == book(states['diameter_moved'])
print('Geometry circular arc passed for Symbolic entry, partial plot, Point On, diameter drag, typed PPL and restart')
