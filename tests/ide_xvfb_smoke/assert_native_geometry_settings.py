"""Exercise Geometry viewport editing, PPL readback, and profile restart."""
import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def load(name):
    report = json.loads((root / f'native_side_{name}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {item['label']: item['state'] for item in report['snapshots']}


def camera(state):
    return state['native_tools']['memory']['model_windows']['Geometry']


states = load('geometry_settings')
reopened = load('geometry_settings_reopen')['reopened']
assert states['initial']['native_tools']['page'] == 'PlotSetup'
initial = camera(states['initial'])
assert [initial[key] for key in ('xmin', 'xmax', 'ymin', 'ymax', 'axes')] == [
    -16, 15.9, -11, 10.9, True
]
rejected = states['rejected']
assert rejected['native_session']['home_input'] == '1/1000'
assert 'Invalid input' in rejected['native_tools']['error']
assert camera(rejected) == initial

edited = states['edited']
assert edited['native_tools']['page'] == 'PlotSetup'
assert edited['native_tools']['error'] is None
assert edited['native_session']['home_input'] == ''
assert [camera(edited)[key] for key in ('xmin', 'xmax', 'ymin', 'ymax', 'axes')] == [
    -16, 47.8, -11, 32.8, False
]
for state in (states['readback'], reopened):
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    assert [float(item['value']) for item in outcome['typed_result']['value']] == [-16, -11, 0.2, 1]
    assert camera(state) == camera(edited)
    assert 'PixSize' not in state['calculator_memory']
print('Geometry viewport editing, invalid size recovery, axes toggle, scoped PPL readback and restart passed')
