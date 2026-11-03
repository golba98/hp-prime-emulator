#!/usr/bin/env python3
"""Check actual IDE snapshots; this script performs no calculator USB I/O."""
import json
import sys
from pathlib import Path

report = json.loads(Path(sys.argv[1]).read_text())
snapshots = {entry['label']: entry['state'] for entry in report['snapshots']}
checks = 0
for name, value in [('first', '112'), ('second', '114'), ('edited', '144')]:
    state = snapshots[name]
    assert state['worker_outcome']['kind'] == 'success', state['worker_outcome']
    assert state['worker_outcome']['value'] == value, state['worker_outcome']
    assert not state['worker_alive']
    checks += 3
state = snapshots['foreign_error']
assert state['worker_outcome']['kind'] == 'error'
errors = [p for p in state['problems'] if p['provenance'] == 'native-runtime']
assert len(errors) == 1, errors
assert errors[0]['file'].startswith('LEAF'), errors
assert errors[0]['line'] == 3, errors
assert any(p['file'].startswith('UNRELATED') for p in state['problems'])
assert report['window_captures'], 'missing actual rendered window'
checks += 6
print(f'{checks} M17 Native linked IDE assertions passed')
