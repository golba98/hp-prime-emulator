#!/usr/bin/env python3
"""Run the retained independent G1 value controls through the actual IDE worker."""
import json
import sys
from pathlib import Path
root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
fixture = Path(__file__).resolve().parents[1] / 'fixtures/native-cas-special-controls.json'
cases = json.loads(fixture.read_text())['cases']
report = json.loads((root / 'native_side_cas_arguments.json').read_text())
assert not report['wait_timeouts'], report['wait_timeouts']
states = {item['label']: item['state'] for item in report['snapshots']}
def display(value, nested=False):
    if isinstance(value, list):
        return '{' + ','.join(display(item, True) for item in value) + '}'
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False) if nested else value
    return str(value)
for case in cases:
    state = states[case['name']]
    outcome = state['worker_outcome']
    assert outcome['kind'] == 'success', (case['name'], outcome)
    assert outcome['value'] == display(case['expected']), (case['name'], outcome, case['expected'])
    assert not state['native_session']['worker_active']
print(f'Native CAS IDE: {len(cases)} independent genuine G1 controls passed through compilation, Run and the production worker')
