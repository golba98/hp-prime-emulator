"""Real Home/PPL Geometry instructions, explicit redraw, plotted triangle and restart."""
import json
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
created = json.loads((root / 'native_side_geometry_bridge.json').read_text())
reopened = json.loads((root / 'native_side_geometry_bridge_reopen.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states = {item['label']: item['state'] for item in created['snapshots']}
restored = {item['label']: item['state'] for item in reopened['snapshots']}
for label, state in list(states.items()) + list(restored.items()):
    assert state['native_tools']['error'] is None, (label, state['native_tools']['error'])
    assert not state['native_tools']['calculating'], label


def book(state):
    return state['native_tools']['memory']['models']['geometry_book']


def scene(state):
    results = state['native_model_results']
    return results.get('Geometry') or results['Geometry:plot']['geometry']


def objects(state):
    return {obj['name']: obj['value']['Ok'] for obj in scene(state)['objects']}


def result(state):
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    assert outcome['result'] == '{5,6,12}', outcome
    values = outcome['typed_result']['value']
    assert [float(value['value']) for value in values] == [5, 6, 12]


expected_sources = ['point(0,0)', 'point(4,0)', 'point(0,3)', 'triangle(GA,GB,GC)', 'distance(GB,GC)']
for state in list(states.values()) + list(restored.values()):
    saved = book(state)
    assert [obj['name'] for obj in saved['objects']] == ['A', 'B', 'C', 'D', 'E']
    assert [obj['source'] for obj in saved['objects']] == expected_sources
    assert 'current_values' not in saved, 'The persisted ToolMemory contains definitions, not an invented computed-value snapshot'
    assert all(name not in state['calculator_memory'] for name in ['GA', 'GB', 'GC', 'GD', 'GE'])
    assert {'GEOBRIDGE', 'GEORESTORE'} <= set(state['calculator_programs'])
result(states['created_symbolic'])
result(restored['reopened_recomputed'])
result(restored['reopened_expr'])
assert 'STARTVIEW' not in restored['reopened_recomputed']['calculator_programs']['GEORESTORE']
assert states['created_symbolic']['native_tools']['page'] == 'Symbolic'
for label, state in [('created_plot', states['created_plot']), ('reopened_plot', restored['reopened_plot'])]:
    assert state['native_tools']['page'] == 'Plot', label
    found = objects(state)
    assert found['A']['Point'] == {'x': 0, 'y': 0}
    assert found['B']['Point'] == {'x': 4, 'y': 0}
    assert found['C']['Point'] == {'x': 0, 'y': 3}
    assert found['D']['Polygon'] == [{'x': 0, 'y': 0}, {'x': 4, 'y': 0}, {'x': 0, 'y': 3}]
    assert found['E']['Number'] == {'re': 5, 'im': 0}
    data = state['native_model_results']['Geometry:plot']
    assert data['curves'][data['labels'].index('GD')] == [[0, 0], [4, 0], [0, 3], [0, 0]]
    # Sample three actual hypotenuse locations away from the axes and labels;
    # dark diagonal pixels distinguish the construction from the gray grid.
    window = state['native_tools']['memory']['model_windows']['Geometry']
    left, top = state['lcd_rect']['min']
    right, bottom = state['lcd_rect']['max']
    image = Image.open(root / f'geometry_bridge_{label}.png').convert('RGB')
    for x, y in [(1, 2.25), (2, 1.5), (3, .75)]:
        px = 1 + 318 * (x - window['xmin']) / (window['xmax'] - window['xmin'])
        py = 218 - 218 * (y - window['ymin']) / (window['ymax'] - window['ymin'])
        sx = left + px / 320 * (right - left)
        sy = top + py / 240 * (bottom - top)
        raw = image.crop((round(sx) - 3, round(sy) - 3, round(sx) + 4, round(sy) + 4)).tobytes()
        assert any(max(pixel) < 100 for pixel in zip(raw[::3], raw[1::3], raw[2::3])), (label, x, y)
assert book(restored['reopened_definitions']) == book(states['created_plot'])
assert book(restored['reopened_plot']) == book(states['created_plot'])
print('Geometry Home/PPL bridge: five real instructions, explicit Symbolic redraw, immediate distance5/area6/perimeter12, rendered triangle and separate-process STARTAPP recomputation without STARTVIEW and EXPR catalog linking passed')
