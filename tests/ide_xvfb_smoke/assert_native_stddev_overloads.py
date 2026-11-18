"""Check the supported scalar and matrix stddev forms through Native Home."""
import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
report = json.loads((root / 'native_side_stddev_overloads.json').read_text())
assert not report['wait_timeouts'], report['wait_timeouts']
snapshot, = (item for item in report['snapshots'] if item['label'] == 'stddev_overloads')
outcome = snapshot['state']['native_session']['last_outcome']
assert outcome['status'] == 'success', outcome
typed = outcome['typed_result']['value']
assert [typed[i]['value'] for i in [0, 1, 2, 4, 5, 6, 7]] == ['0', '0', '4', '8', 'undef', '6', '{undef}']
assert typed[3]['type'] == 'matrix' and typed[3]['vector'], typed[3]
assert [item['value'] for item in typed[3]['value']] == ['0', '3.46410161514']
print('Native stddev matches typed scalar, weighted matrix and undefined G1 outcomes')
