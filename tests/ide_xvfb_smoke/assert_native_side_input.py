#!/usr/bin/env python3
"""Behavior checks for the real desktop Native calculator input workflow."""
import json
import sys
from pathlib import Path

path = Path(sys.argv[1] if len(sys.argv) > 1 else '/tmp/ppl-ide-e2e/native_side_input.json')
data = json.loads(path.read_text())
assert data['wait_timeouts'] == [], data['wait_timeouts']
states = {item['label']: item['state'] for item in data['snapshots']}
native = {name: state['native_session'] for name, state in states.items()}
expected = {
    'fraction': '(1)/(2)',
    'fraction_pointer_edit': '(1)/(2)',
    'cursor_paste': '132',
    'inverse_sin': 'ASIN()',
    'space_ans': ' Ans',
    'alpha_lock': 'ABC',
    'older_history': '1+1',
    'history_draft': '',
}
for label, expression in expected.items():
    assert native[label]['home_input'] == expression, (label, native[label]['home_input'])
assert native['fraction_result']['last_outcome']['result'] == '0.5'
assert native['cursor_paste']['cursor'] == 2
assert native['inverse_sin']['cursor'] == 5
assert len(native['older_history']['history']) == 4
assert native['catalog_before_tap']['selected'] == 1
assert native['catalog_after_tap']['selected'] == 0
assert native['catalog_after_tap']['catalog'][0]['name'] == 'FIRST'
assert native['on_interrupt']['last_outcome']['status'] == 'interrupted'
assert not native['on_interrupt']['worker_active']
assert states['on_interrupt']['vm_state'] == 'Idle'
assert native['after_interrupt']['last_outcome']['result'] == '7'
assert states['after_interrupt']['vm_state'] == 'Done'
print('Native side input: 21 behavioral assertions passed')
