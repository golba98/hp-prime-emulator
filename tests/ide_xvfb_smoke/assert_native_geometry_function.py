"""Native Geometry function graphs, closest-point input and dependency redraw."""
import json
import math
from pathlib import Path
import sys

from PIL import Image

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
GRAPH = 'plotfunc(x^2/4+abscissa(GA))'


def snapshots(suffix=''):
    path = ROOT / f'native_side_geometry_function{suffix}.json'
    report = json.loads(path.read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {item['label']: item['state'] for item in report['snapshots']}


def book(state):
    return state['native_tools']['memory']['models']['geometry_book']


def shapes(state):
    scene = state['native_model_results']['Geometry:plot']['geometry']
    return {obj['name']: obj['value']['Ok'] for obj in scene['objects']}


def outcome(state):
    value = state['native_session']['last_outcome']
    assert value['status'] == 'success'
    assert value['typed_result']['type'] == 'list'
    return value['typed_result']['value']


def number(item):
    assert item['type'] == 'real', item
    return float(item['value'])


def symbolic(item):
    assert item['type'] == 'symbolic', item
    return item['value']['value']


def ink(label, state, x, y):
    window = state['native_tools']['memory']['model_windows']['Geometry']
    px = 319 * (x - window['xmin']) / (window['xmax'] - window['xmin'])
    py = 219 - 219 * (y - window['ymin']) / (window['ymax'] - window['ymin'])
    left, top = state['lcd_rect']['min']
    right, bottom = state['lcd_rect']['max']
    sx = round(left + px * (right - left) / 320)
    sy = round(top + py * (bottom - top) / 240)
    with Image.open(ROOT / f'geometry_function_{label}.png').convert('RGB') as image:
        colors = {(0, 0, 0), (255, 128, 128), (0, 191, 255)}
        return any(
            image.getpixel((sx + dx, sy + dy)) in colors
            for dx in range(-2, 3) for dy in range(-2, 3)
        )


states = snapshots()
restored = snapshots('_reopen')['reopened']
assert [o['source'] for o in book(states['prepared'])['objects']] == ['point(1,0)']
assert [o['source'] for o in book(states['curve_created'])['objects']] == ['point(1,0)', GRAPH]
assert states['function_entry']['native_tools']['page'] is None
for label, state in [*states.items(), ('reopened', restored)]:
    if label == 'function_entry':
        continue
    tools = state['native_tools']
    assert tools['page'] == 'Plot' and not tools['calculating'] and tools['error'] is None, label

assert not ink('prepared', states['prepared'], -5., 7.25)
for label in ('curve_created', 'point_created', 'point_moved'):
    assert ink(label, states[label], -5., 7.25), label
for label, state in [('redrawn', states['redrawn']), ('reopened', restored)]:
    assert ink(label, state, -5., 9.25), label
    assert not ink(label, state, -5., 7.25), label

def constrained(state, shift):
    p = shapes(state)['C']['Point']
    assert math.isclose(p['y'], p['x'] ** 2 / 4 + shift, abs_tol=1e-10), p
    source = book(state)['objects'][2]['source']
    assert source.startswith('element(GB,') and source.endswith(')'), source
    assert math.isclose(float(source[len('element(GB,'):-1]), p['x']), source
    return p


clicked = constrained(states['point_created'], 1)
moved = constrained(states['point_moved'], 1)
assert abs(clicked['x'] - 2.23959320696) < 0.008, clicked
assert abs(moved['x'] - 4.) < 0.12 and moved['x'] > clicked['x'] + 1., moved
redrawn = states['redrawn']
for state in (redrawn, restored):
    p = constrained(state, 3)
    assert p['x'] == moved['x']
    paths = shapes(state)['B']['Function']['paths']
    assert len(paths) == 1 and len(paths[0]) > 100
    assert all(math.isclose(sample['y'], sample['x'] ** 2 / 4 + 3.) for sample in paths[0])
    assert [o['source'] for o in book(state)['objects']][3:] == ['plotfunc(x^2/4-12)']

values = outcome(redrawn)
assert [number(v) for v in values[:2]] == [8, 8]
assert number(values[2]) == number(values[3]) == moved['y']
assert number(values[4]) == moved['x']
assert number(values[5]) == constrained(redrawn, 3)['y']
assert [number(v) for v in values[6:9]] == [-30., 4., -3.]
assert symbolic(values[9]) == GRAPH
values = outcome(restored)
point = constrained(restored, 3)
assert [number(v) for v in values[:-1]] == [8, 8, point['x'], point['y'], 4., -3.]
assert symbolic(values[-1]) == GRAPH
assert book(restored) == book(redrawn)
print('Geometry Function passed for graph entry, constrained drag, drawing, typed PPL and restart')
