"""Point On through the live Geometry canvas, Home/PPL and profile reopen."""
import json
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def snapshots(suffix=''):
    report = json.loads((root / f'native_side_geometry_curve_point{suffix}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {entry['label']: entry['state'] for entry in report['snapshots']}


def construction(state):
    return state['native_tools']['memory']['models']['geometry_book']


def plot_objects(state):
    scene = state['native_model_results']['Geometry:plot']['geometry']
    return {item['name']: item['value']['Ok'] for item in scene['objects']}


def outcome(state):
    result = state['native_session']['last_outcome']
    assert result['status'] == 'success', result
    assert result['typed_result']['type'] == 'list', result
    return result['typed_result']['value']


def definition(state, name):
    saved = construction(state)
    assert 'current_values' not in saved, saved
    item, = (item for item in saved['objects'] if item['name'] == name)
    return item['source']


def verify_rendered_point(label, state, name, screen_offset):
    window = state['native_tools']['memory']['model_windows']['Geometry']
    x, y = (plot_objects(state)[name]['Point'][axis] for axis in ('x', 'y'))
    pixel_x = 319 * (x - window['xmin']) / (window['xmax'] - window['xmin'])
    pixel_y = 219 - 219 * (y - window['ymin']) / (window['ymax'] - window['ymin'])
    pixel_x += screen_offset[0]
    pixel_y += screen_offset[1]
    left, top = state['lcd_rect']['min']
    right, bottom = state['lcd_rect']['max']
    screen_x = round(left + pixel_x * (right - left) / 320)
    screen_y = round(top + pixel_y * (bottom - top) / 240)
    image = Image.open(root / f'geometry_curve_point_{label}.png').convert('RGB')
    neighborhood = image.crop((screen_x - 1, screen_y - 1, screen_x + 2, screen_y + 2))
    assert any(
        max(neighborhood.getpixel((x, y))) < 110 for x in range(3) for y in range(3)
    ), (label, name)


states = snapshots()
restored = snapshots('_reopen')['fresh_process']
for label, state in [*states.items(), ('fresh_process', restored)]:
    tools = state['native_tools']
    assert tools['page'] == 'Plot', (label, tools['page'])
    assert tools['error'] is None, (label, tools['error'])
    assert not tools['calculating'], label

prepared = outcome(states['prepared'])
assert [float(item['value']) for item in prepared] == [1, 6], prepared

for label, source, x in (
    ('segment_point', 'element(GC,0.525)', 2.1),
    ('dragged_on_segment', 'element(GC,0.75)', 3),
    ('dependency_redraw', 'element(GC,0.75)', 6),
):
    assert definition(states[label], 'E') == source, label
    assert plot_objects(states[label])['E']['Point'] == {'x': x, 'y': 0}, label
assert definition(states['circle_point'], 'F') == 'element(GD,0)'
assert plot_objects(states['circle_point'])['F']['Point'] == {'x': 8, 'y': 4}

for state in (states['dependency_redraw'], restored):
    values = outcome(state)
    assert [float(item['value']) for item in values[:4]] == [6, 0, 8, 4], values
    assert all(item['type'] == 'symbolic' for item in values[4:]), values
    assert [item['value']['value'] for item in values[4:]] == [
        'element(GC,0.75)', 'element(GD,0)'
    ], values
    assert plot_objects(state)['E']['Point'] == {'x': 6, 'y': 0}
    assert plot_objects(state)['F']['Point'] == {'x': 8, 'y': 4}
    assert definition(state, 'B') == 'point(8,0)'

assert construction(restored) == construction(states['dependency_redraw'])
verify_rendered_point('dependency_redraw', states['dependency_redraw'], 'E', (0, -2))
verify_rendered_point('fresh_process', restored, 'F', (2, 0))
print('Geometry Point On passed for canvas placement, constrained drag, PPL redraw and restart')
