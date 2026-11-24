#!/usr/bin/env python3
"""Real desktop math-node deletion and Characters page selection/resumption."""
import json
from pathlib import Path

root = Path('/tmp/ppl-ide-e2e')
data = json.loads((root / 'native_side_editing.json').read_text())
assert data['wait_timeouts'] == [], data['wait_timeouts']
states = {s['label']: s['state'] for s in data['snapshots']}
for name, text in {
    'keypad_sin': 'SIN()', 'keypad_sin_deleted': '',
    'undo_sin': 'SIN()', 'redo_delete_sin': '',
    'typed_sin': 'sin()', 'typed_sin_deleted': '',
    'characters_open': '2', 'pi_inserted': '2*π',
}.items():
    assert states[name]['native_session']['home_input'] == text, name
assert states['undo_sin']['native_session']['cursor'] == 4
assert states['greek_selected']['native_tools']['memory']['characters'] == {'page': 7, 'code': 0x370}
assert states['pi_selected']['native_tools']['memory']['characters'] == {'page': 7, 'code': 0x3c0}
assert states['pi_inserted']['native_tools']['page'] is None
assert states['pi_evaluated']['native_session']['last_outcome']['result'] == '6.28318530718'
assert states['characters_reopened']['native_tools']['memory']['characters'] == {'page': 7, 'code': 0x3c0}
reopened = json.loads((root / 'native_side_editing_reopen.json').read_text())
assert reopened['wait_timeouts'] == [], reopened['wait_timeouts']
restored = {s['label']: s['state'] for s in reopened['snapshots']}
assert restored['restored_characters']['native_tools']['memory']['characters'] == {'page': 7, 'code': 0x3c0}
assert restored['restored_pi_inserted']['native_session']['home_input'] == 'π'
assert restored['restored_pi_inserted']['native_tools']['page'] is None
print('Native editing: 19 assertions passed, including a separate GUI-process reopen')
