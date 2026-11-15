#!/usr/bin/env python3
"""Exercise independent G1 EXPR controls through the IDE, keyboard and reopen."""
import json
import sys
from pathlib import Path
root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
controls = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-expr-catalog-controls.json').read_text())

def states(name):
    data = json.loads((root / f'native_side_{name}.json').read_text())
    assert not data['wait_timeouts'], data['wait_timeouts']
    return {item['label']: item['state'] for item in data['snapshots']}

def display(value, nested=False):
    if isinstance(value, list):
        return '{' + ','.join(display(item, True) for item in value) + '}'
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False) if nested else value
    return str(value)

created = states('expr_catalog')
for case in controls['cases']:
    state = created[case['name']]
    assert state['worker_outcome']['kind'] == 'success', (case['name'], state['worker_outcome'])
    assert state['worker_outcome']['value'] == display(case['expected']), (case['name'], state['worker_outcome'])
    assert not state['native_session']['worker_active']
assert created['input-typed']['native_session']['active_dialog']['kind'] == 'input'
assert created['input-typed']['native_session']['owner'] == 'CalculatorModal'
assert created['input-result']['worker_outcome']['value'] == '{3,119}'
assert created['home-call']['native_session']['history'][-1]['result'] == '120'
assert created['unsaved-call']['native_session']['history'][-1]['result'] == '504'
assert all(not row['error'] for row in created['unsaved-call']['native_session']['history'])
for program in controls['programs']:
    assert created['unsaved-call']['calculator_programs'][program['name']] == program['source']
assert 'UNSAVEDLIB' not in created['unsaved-call']['calculator_programs']
assert 'count' not in created['unsaved-call']['calculator_memory']
assert 'counter' not in created['unsaved-call']['calculator_memory']
reopened = states('expr_catalog_reopen')['home-reopened']
assert reopened['native_session']['history'][-1]['result'] == '121'
assert reopened['worker_outcome']['kind'] == 'success'
assert reopened['calculator_programs'] == created['unsaved-call']['calculator_programs']
assert 'count' not in reopened['calculator_memory']
print(f"Native EXPR: {len(controls['cases'])} independent G1 controls, suspended INPUT, Home, unsaved IDE export and separate-process private-memory restore passed")
