#!/usr/bin/env python3
"""Desktop history interaction, structural Show and separate-process persistence."""
import json
from pathlib import Path

root = Path('/tmp/ppl-ide-e2e')
data = json.loads((root / 'native_side_history.json').read_text())
assert data['wait_timeouts'] == [], data['wait_timeouts']
n = {s['label']: s['state']['native_session'] for s in data['snapshots']}
assert n['selected_result']['home_input'] == '4*'
assert n['selected_result']['history_result'] is True
assert n['copied_result']['home_input'] == '4*3'
assert n['evaluated_copy']['last_outcome']['result'] == '12'
assert n['matrix_result']['history'][-1]['typed_result']['rows'] == 12
assert n['matrix_result']['history'][-1]['typed_result']['cols'] == 12
for label in ['show_matrix', 'viewer_ignores_typing', 'scrolled_matrix', 'show_closed']:
    assert n[label]['home_input'] == ''
    assert n[label]['ans'] == n['matrix_result']['ans']
    assert n[label]['run_id'] == n['matrix_result']['run_id']
assert n['show_matrix']['history_viewer'] == {'x': 0, 'y': 0}
assert n['scrolled_matrix']['history_viewer']['x'] > 0
assert n['scrolled_matrix']['history_viewer']['y'] > 0
assert n['show_closed']['history_viewer'] is None
assert n['selected_expression']['home_input'] == '7+'
assert n['selected_expression']['history_result'] is False
assert n['oldest_expression']['history_position'] == 0
assert n['copied_expression']['home_input'] == '7+1+2'
assert n['final_result']['last_outcome']['result'] == '10'
typed = n['nested_result']['history'][-1]['typed_result']
assert typed['type'] == 'list'
assert typed['value'][0]['type'] == 'matrix'
assert typed['value'][1] == {'type': 'string', 'value': 'SQRT(9)', 'display': '"SQRT(9)"'}
assert typed['value'][2]['display'] == '#FFh'
assert n['deleted_result']['home_input'] == '99*'
assert n['deleted_result']['history'][0]['result_deleted'] is True
assert n['deleted_result']['history_result'] is False
assert n['deleted_result']['ans'] == n['nested_result']['ans']
assert n['deleted_expression']['home_input'] == '99*'
assert len(n['deleted_expression']['history']) == len(n['nested_result']['history'])-1
reopened = json.loads((root / 'native_side_history_reopen.json').read_text())
assert reopened['wait_timeouts'] == [], reopened['wait_timeouts']
r = {s['label']: s['state']['native_session'] for s in reopened['snapshots']}
assert r['restored']['history'][-1]['typed_result'] == typed
assert r['restored']['history'][0]['result_deleted'] is True
assert r['restored']['history'][0]['expression'] == '4*3'
assert r['restored']['home_input'] == '99*'
assert r['restored_show']['history_viewer'] == {'x': 0, 'y': 0}
assert r['restored_show']['ans'] == r['restored']['ans']
print('Native history: draft/copy/evaluation, 12×12 Show scrolling, typed nesting and process reopen passed')
