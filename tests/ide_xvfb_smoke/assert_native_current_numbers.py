"""Current GUI profiles preserve exact history objects and original arithmetic modes."""
import json
import sys
from pathlib import Path
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
created=json.loads((root/'native_side_current_numbers.json').read_text())
reopened=json.loads((root/'native_side_current_numbers_reopen.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states={s['label']:s['state'] for s in created['snapshots']}
restored={s['label']:s['state'] for s in reopened['snapshots']}
for label,state in list(states.items())+list(restored.items()):
    assert state['calculator_numeric_version']==15751,label
    assert state['native_tools']['error'] is None,(label,state['native_tools']['error'])
    assert not state['worker_alive'] and not state['native_tools']['calculating'],label
def outcome(label):return states[label]['native_session']['last_outcome']
def typed(label):return outcome(label)['typed_result']
long_origin={'kind':'rational','numerator':'1234567890123456789','denominator':'1','negative':False}
assert typed('long')['origin']==long_origin
assert states['long_copy']['native_session']['home_input']=='1.234567890123456789e18'
assert outcome('long_difference')['result']=='1'
assert states['list_copy']['native_session']['home_input']=='{(1/3),(1*π/7),approx(0.25),#FF:-8h}'
assert typed('list_recalled')==typed('list')
items=typed('list')['value']
assert items[0]['origin']=={'kind':'rational','numerator':'1','denominator':'3','negative':False}
assert items[1]['origin']=={'kind':'pi_multiple','numerator':'1','denominator':'7','negative':False}
assert items[2]['origin']=={'kind':'decimal'}
assert items[3]['word']=={'kind':'integer','base':16,'bits':8,'signed_word':True,'value':'255'}
# The original numeric matrix constructor makes decimal cells, unlike a list.
assert typed('matrix')['rows']==typed('matrix')['cols']==2
assert all(v['origin']=={'kind':'decimal'} for v in typed('matrix')['value'])
assert typed('matrix_recalled')==typed('matrix')
assert states['matrix_copy']['native_session']['home_input']=='[[approx(0.333333333333),approx(0.448798950513)],[approx(1.23456789012E18),approx(0.25)]]'
assert typed('overflow_ppl')['value'][0]['value']=='9.99999999999E499'
rows=states['overflow_numeric']['native_numeric_values']
assert [[v['value'] for v in row] for row in rows[:2]]==[['1E400','9.99999999999E499'],['1.1E400','9.99999999999E499']]
assert outcome('modes')['result']=='{0,3,1,1,1,1}'
assert states['final']['native_session']['home_input']=='12+'
assert restored['reopened']['native_session']['home_input']=='12+'
assert restored['reopened_difference']['native_session']['last_outcome']['result']=='1'
assert restored['reopened_final']['native_session']['home_input']=='12+'
# Serialized history must retain every precise object after a separate process.
original={entry['expression']:entry for entry in states['final']['native_session']['history']}
loaded={entry['expression']:entry for entry in restored['reopened']['native_session']['history']}
for expression in ['1234567890123456789','{1/3,π/7,approx(0.25),#FF:-8h}']:
    assert loaded[expression]['typed_result']==original[expression]['typed_result'],expression
print('Current Native: fresh15751 profile, exact19/rational/π/decimal/word history copy and restart, matrix decimal identity, original complex modes and signed finite overflow passed')
