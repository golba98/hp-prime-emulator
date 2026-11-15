"""Actual GUI field edits retain BCD values beyond binary floating point."""
import json
import sys
from pathlib import Path
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
created=json.loads((root/'native_side_function_numeric_exact.json').read_text())
reopened=json.loads((root/'native_side_function_numeric_exact_reopen.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states={s['label']:s['state'] for s in created['snapshots']}
restored={s['label']:s['state'] for s in reopened['snapshots']}
for label,state in list(states.items())+list(restored.items()):
    assert state['native_tools']['error'] is None,(label,state['native_tools']['error'])
    assert not state['worker_alive'] and not state['native_tools']['calculating'],label
    assert not any(k in state['calculator_memory'] for k in ['NumStart','NumStep','NumZoom','NumType','NumIndep']),label
    assert 'F1' not in state['calculator_programs'],label
for label,start,step in [('big_setup','1E400','1E399'),('big','1E400','1E399'),('tiny_setup','1E-400','1E-401'),('tiny','1E-400','1E-401')]:
    settings=states[label]['calculator_app_numeric_settings']['Function']
    assert (settings['start'],settings['step'])==(start,step),(label,settings)
    assert states[label]['native_session']['home_input']=='12+',label
for label,expected in [('big',[['1E400','1'],['1.1E400','1.1']]),('tiny',[['1E-400','1'],['1.1E-400','1.1']])]:
    values=states[label]['native_numeric_values'][:2]
    assert [[v['value'] for v in row] for row in values]==expected,(label,values)
    assert all(v['type']=='real' for row in values for v in row)
    # Float samples are an observation projection; rendering uses typed values.
    assert states[label]['native_tools']['table'][0][1]==1
complex_cells=states['complex_numeric']['native_numeric_values']
assert complex_cells[0][1]['type']=='complex' and complex_cells[0][1]['imaginary']=='1'
assert states['complex_home']['native_session']['last_outcome']['typed_result']['real']=='2'
assert states['complex_home']['native_session']['last_outcome']['typed_result']['imaginary']=='1'
assert states['zero_division']['native_numeric_values'][0][1] is None
assert states['zero_division']['native_numeric_values'][1][1]['value']=='10'
assert states['scope']['native_session']['last_outcome']['result']=='{{1,1},{0,0.1,2,0},{1,1}}'
assert states['scope']['calculator_app_numeric_settings']['Parametric']['start']=='7'
assert states['scope']['calculator_app_numeric_settings']['Parametric']['step']=='0.5'
for label,state in restored.items():
    assert state['calculator_app_numeric_settings']['Function']==states['tiny']['calculator_app_numeric_settings']['Function'],label
assert restored['reopened_tiny']['native_numeric_values'][:2]==states['tiny']['native_numeric_values'][:2]
print('Function Numeric exact: actual big/tiny BCD field edits and table values, isolated per-app VM settings, no ordinary Num globals, Home draft and process restart passed')
