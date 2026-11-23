"""Select a calculated point, arm Trace, and drag its parent on the LCD."""
import json
import sys
from pathlib import Path
from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
report = json.loads((root / 'native_side_geometry_dependent_trace.json').read_text())
assert not report['wait_timeouts']
states = {item['label']: item['state'] for item in report['snapshots']}
for state in states.values():
    assert state['native_tools']['error'] is None
    assert not state['native_tools']['calculating']
def objects(state):
    return state['native_tools']['memory']['models']['geometry_book']['objects']

outcome = states['prepared']['native_session']['last_outcome']
assert outcome['status'] == 'success', outcome
assert [float(item['value']) for item in outcome['typed_result']['value']] == [0, 4, 4]
assert objects(states['armed'])[1]['traced']
moved = states['moved']
a, b = objects(moved)[:2]
assert b['source'] == 'point(abscissa(GA)+3,ordinate(GA))'
assert not a.get('trace_samples')
samples = b['trace_samples']
assert len(samples) >= 3, samples
assert samples[0] == {'x': 4., 'y': 2.}
assert samples[-1] == {'x': 8.2, 'y': -2.6}
assert moved['native_tools']['plot_points'][:2] == [[[5.2, -2.6]], [[8.2, -2.6]]]
with Image.open(root / 'dependent_trail.png') as screenshot:
    assert any(color == (23, 122, 153) for _, color in screenshot.convert('RGB').getcolors(screenshot.width * screenshot.height))
print('Dependent point Trace followed its parent drag and painted the evaluated trail')
