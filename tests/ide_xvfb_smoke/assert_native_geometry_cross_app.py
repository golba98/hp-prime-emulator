"""Desktop controls for retained Geometry references in Spreadsheet and Function."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def read(scenario):
    report = json.loads((root / f'native_side_{scenario}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {entry['label']: entry['state'] for entry in report['snapshots']}


def values(state):
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    return [float(item['value']) for item in outcome['typed_result']['value']]


creation = read('geometry_cross_app')
reopen = read('geometry_cross_app_reopen')
assert values(creation['initial_values']) == [8, 7, 7]
assert values(creation['redrawn_values']) == [7, 12, 12]
assert values(reopen['reopened_values']) == [8, 12, 12]

for label, state, value in [
    ('initial', creation['sheet_value'], 7),
    ('redrawn', creation['sheet_updated'], 12),
    ('reopened', reopen['sheet_reopened'], 12),
]:
    assert state['native_tools']['page'] == 'Numeric', label
    assert state['native_tools']['error'] is None, label
    assert state['native_model_results']['Spreadsheet']['A1'] == {'Number': value}, label
    cells = state['native_tools']['memory']['models']['spreadsheet']['cells']
    assert cells['A1'].startswith('=') and 'Geometry.GC' in cells['A1'], cells
    assert 'GC' not in state['calculator_memory']

plot = creation['function_plot']['native_tools']
assert plot['page'] == 'Plot' and plot['error'] is None
points, = plot['plot_points']
assert len(points) >= 100
assert all(math.isclose(y, x + 10, abs_tol=1e-8) for x, y in points)

numeric = creation['function_numeric']
assert numeric['native_tools']['page'] == 'Numeric' and numeric['native_tools']['error'] is None
assert len(numeric['native_numeric_values']) > 30
assert [float(cell['value']) for cell in numeric['native_numeric_values'][0]] == [0, 10]
print('Geometry references recalculate in the real worksheet, Function plot and table, and after process restart')
