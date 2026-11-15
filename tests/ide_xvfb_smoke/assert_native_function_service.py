"""Actual UI definitions used by Home, PPL, EXPR and CAS, then reopened."""
import json
import math
import sys
from pathlib import Path
root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
created = json.loads((root/'native_side_function_service.json').read_text())
reopened = json.loads((root/'native_side_function_service_reopen.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states = {s['label']: s['state'] for s in created['snapshots']}
restored = {s['label']: s['state'] for s in reopened['snapshots']}
for label, state in list(states.items()) + list(restored.items()):
    assert state['native_tools']['error'] is None, (label, state['native_tools']['error'])
    assert not state['worker_alive'] and not state['native_tools']['calculating'], label
    assert not any(k in state['calculator_memory'] for k in ['F1', 'F2', 'F3']), label
    assert not any(k in state['calculator_programs'] for k in ['F1', 'F2', 'F3']), label

def result(label):
    outcome = states[label]['native_session']['last_outcome']
    assert outcome['status'] == 'success', (label, outcome)
    return outcome['result']

for label, expected in [('direct','{1,5,7,99}'), ('types','{5,0,65026,0,{2,5},6}'), ('expr','12'), ('sheet_default','{"","",2,42,42}'), ('caller_sheet','{5,"FN_CELL2","FN_CELL","FN_CELL",99}')]:
    assert result(label) == expected, (label, result(label))
complex_value = states['complex']['native_session']['last_outcome']['typed_result']
assert complex_value['type'] == 'complex' and complex_value['real'] == '-2' and complex_value['imaginary'] == '4'
assert states['cas_bare']['calculator_cas']['last_value'] == {'type':'integer','value':'8'}
assert states['cas_qualified']['calculator_cas']['last_value'] == {'type':'integer','value':'6'}
for label in ['cas_bare','cas_qualified']:
    assert states[label]['calculator_memory']['Ans'] == states['cas_before']['calculator_memory']['Ans'], label
angle = states['caller_angle']['native_session']['last_outcome']['typed_result']['value']
assert math.isclose(float(angle[0]['value']), .987688340595, rel_tol=1e-12)
assert [v['value'] for v in angle[1:]] == ['1','3','99']
assert states['caller_angle']['calculator_home_settings']['angle'] == 'Degrees'
for state in [states['final'], restored['reopened']]:
    memory = state['native_tools']['memory']
    assert [c['expression'] for c in memory['graphs']['Function']['curves'][:3]] == ['X^2+1','F1(X)+2','SIN(X)']
    assert memory['models']['spreadsheet']['cells']['F1'] == '42'
    assert memory['models']['spreadsheet']['names']['FN_CELL'] == {'left':6,'right':6,'top':1,'bottom':1}
    assert state['calculator_memory']['X']['value'] == '99'
assert restored['reopened_function']['native_session']['last_outcome']['result'] == '{5,7,99}'
assert restored['reopened_sheet']['native_session']['last_outcome']['result'] == '{42,"FN_CELL",5}'
print('Function calls: actual edited rows, Home/PPL/EXPR/CAS, typed lists/complex/words, caller namespace and angle, Home preservation and separate-process restoration passed')
