"""Verify Native line intersections with circular arcs and ellipses."""
import json
import math
from pathlib import Path
import sys

from PIL import Image, ImageChops


ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def snapshots(suffix=''):
    path = ROOT / f'native_side_geometry_curve_intersection{suffix}.json'
    report = json.loads(path.read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {item['label']: item['state'] for item in report['snapshots']}


def book(state):
    return state['native_tools']['memory']['models']['geometry_book']


def shape(state, name):
    scene = state['native_model_results']['Geometry:plot']['geometry']
    return next(obj['value']['Ok'] for obj in scene['objects'] if obj['name'] == name)


def typed(state):
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    return outcome['typed_result']


def contents(value):
    assert value['type'] == 'list', value
    return value['value']


def symbol(value):
    assert value['type'] == 'symbolic' and value['value']['type'] == 'symbolic', value
    return value['value']['value']


def typed_point(value):
    expression = symbol(value)
    assert expression.startswith('point(') and expression.endswith(')'), expression
    x, y = expression[len('point('):-1].split(',')
    return float(x), float(y)


def close_point(actual, expected):
    assert math.dist(tuple(actual), expected) < 1e-8, (actual, expected)


def point(state, name, expected):
    value = shape(state, name)['Point']
    close_point([value['x'], value['y']], expected)


def point_mark(before, after, state, x, y):
    window = state['native_tools']['memory']['model_windows']['Geometry']
    px = 319 * (x - window['xmin']) / (window['xmax'] - window['xmin'])
    py = 219 - 219 * (y - window['ymin']) / (window['ymax'] - window['ymin'])
    left, top = state['lcd_rect']['min']
    right, bottom = state['lcd_rect']['max']
    sx = round(left + px * (right - left) / 320)
    sy = round(top + py * (bottom - top) / 240)
    box = (sx - 7, sy - 7, sx + 8, sy + 8)
    with Image.open(ROOT / f'geometry_curve_intersection_{before}.png') as a, \
            Image.open(ROOT / f'geometry_curve_intersection_{after}.png') as b:
        assert ImageChops.difference(a.convert('RGB').crop(box),
                                     b.convert('RGB').crop(box)).getbbox(), (before, after)


states = snapshots()
reopened = snapshots('_reopen')['reopened']
for label, state in [*states.items(), ('reopened', reopened)]:
    tools = state['native_tools']
    page = 'Symbolic' if label == 'symbolic' else 'Plot'
    assert tools['page'] == page and tools['error'] is None and not tools['calculating'], label

before = (math.sqrt(12), 2.)
ellipse_left = (-math.sqrt(13), -5.)
ellipse_right = (math.sqrt(13), -5.)
after = (math.sqrt(32), 2.)
point(states['arc_hit'], 'G', before)
point(states['ellipse_hit'], 'H', ellipse_left)
point_mark('prepared', 'arc_hit', states['arc_hit'], *before)
point_mark('arc_hit', 'ellipse_hit', states['ellipse_hit'], *ellipse_left)
assert len(book(states['prepared'])['objects']) == 6
assert len(book(states['arc_hit'])['objects']) == 7
entries = book(states['symbolic'])['objects']
assert entries[-2]['source'].startswith('single_inter(GC,GD,point(')
assert entries[-1]['source'].startswith('single_inter(GE,GF,point(')

for label, state in [('dependency_moved', states['dependency_moved']),
                     ('restored', states['restored']), ('reopened', reopened)]:
    point(state, 'B', (6., 0.))
    point(state, 'G', after)
    point(state, 'H', ellipse_left)
    assert book(state)['objects'][-2:] == entries[-2:], label
missing = states['missing']
assert shape(missing, 'G') == 'Undefined'
assert book(missing)['objects'][-2:] == entries[-2:]
miss_type, miss, empty_list = contents(typed(missing))
assert miss_type['type'] == 'real' and float(miss_type['value']) == 8.
assert miss['type'] == 'symbolic' and miss['value'] == {'type': 'undefined'}
assert contents(empty_list) == []
close_point(typed_point(typed(states['restored'])), after)

assert typed(states['values']) == typed(reopened)
ppl = contents(typed(reopened))
assert all(x['type'] == 'real' for x in ppl[:6])
expected = [8., *after, 8., *ellipse_left]
assert all(abs(float(x['value']) - y) < 1e-8 for x, y in zip(ppl[:6], expected))
arc_hits, ellipse_hits = contents(ppl[6]), contents(ppl[7])
assert len(arc_hits) == 1 and len(ellipse_hits) == 2
close_point(typed_point(arc_hits[0]), after)
for actual, expected_point in zip(ellipse_hits, [ellipse_right, ellipse_left]):
    close_point(typed_point(actual), expected_point)
close_point(typed_point(ppl[8]), ellipse_right)
assert ppl[9]['type'] == 'real' and float(ppl[9]['value']) == 6.
assert [symbol(v) for v in ppl[10:]] == [entry['source'] for entry in entries[-2:]]
assert book(reopened) == book(states['restored'])
print('Geometry curve intersections passed for arc and ellipse input, plot, typed PPL, dependency changes and restart')
