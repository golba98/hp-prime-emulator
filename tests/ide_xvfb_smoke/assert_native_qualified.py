"""Production typed worksheet access across Home/PPL/EXPR/CAS and real dialogs."""
import json
import math
import sys
from pathlib import Path
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
created=json.loads((root/'native_side_qualified.json').read_text())
reopened=json.loads((root/'native_side_qualified_reopen.json').read_text())
states={s['label']:s['state'] for s in created['snapshots']}
restored={s['label']:s['state'] for s in reopened['snapshots']}
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
for label,state in states.items():
    assert state['native_tools']['error'] is None,(label,state['native_tools']['error'])
    assert not state['native_tools']['calculating'],label
def result(label):return states[label]['native_session']['last_outcome']['result']
for label,value in [('home_read','50'),('ppl_live','{18,18,8,-1,255}'),('formula_definition','SUM(B1:B2)'),('copied_definition','9'),('format_attribute','1'),('input_cancelled','{0,50}'),('formula_still_live','100'),('input_confirmed','{1,42}'),('confirmed_value_frozen','42'),('choose_cancelled','{0,2}'),('choose_confirmed','{3,3}'),('tiny_bcd','1')]:assert result(label)==value,(label,result(label))
assert math.isclose(float(result('target_app_angle')),math.sin(90),rel_tol=1e-12)
angle=states['target_app_angle'];assert angle['calculator_memory']['A']['value']=='90'
assert angle['native_tools']['memory']['selected_app']=='Function'
ppl=states['ppl_target_angle']['native_session']['last_outcome']['typed_result']['value']
assert [float(x['value']) for x in ppl[:2]]==[1,3]
for x in ppl[2:]:assert math.isclose(float(x['value']),math.sin(90),rel_tol=1e-12)
assert states['formula_definition']['native_session']['last_outcome']['typed_result']['type']=='symbolic'
assert states['cas_read']['calculator_cas']['last_value']=={'type':'integer','value':'10'}
assert states['cas_before']['calculator_memory']['Ans']==states['cas_read']['calculator_memory']['Ans']
assert states['input_cancel_open']['native_session']['active_dialog']['kind']=='input'
cancel=states['input_cancelled']['native_tools']['memory']['models']['spreadsheet']
assert cancel['cells']['A1']=='=P*5' and 'A1' not in cancel.get('entered_values',{})
choose=states['choose_cancelled']['native_tools']['memory']['models']['spreadsheet']
assert choose['cells']['E1']=='=2'
for state in [states['grid'],restored['reopened']]:
    sheet=state['native_tools']['memory']['models']['spreadsheet']
    assert sheet['cells']['A1']=='42' and sheet['cells']['E1']=='3'
    assert sheet['cells']['A2']=='=A*2' and sheet['cells']['D1']=='=SUM(B1:B2)'
    assert sheet['entered_values']['C1']=={'kind':'integer','base':16,'bits':8,'signed_word':True,'value':'255'}
    assert sheet['entered_values']['G1']=={'kind':'integer','base':16,'bits':64,'value':'18446744073709551615'}
    assert sheet['entered_values']['F1']=={'kind':'real','value':'1E-400','origin':{'kind':'decimal'}}
    values=state['native_model_results']['Spreadsheet']
    for cell,n in [('A1',42),('A2',18),('A3',9),('B1',5),('B2',4),('D1',9),('E1',3)]:assert values[cell]['Number']==n,(cell,values[cell])
    assert values['F1']['Object']['typed']['value']=='1E-400'
    assert state['calculator_memory']['P']['value']=='30'
    assert not any(name.startswith('Spreadsheet.') for name in state['calculator_memory'])
assert states['word_set']['native_session']['last_outcome']['typed_result']['word']==states['word_read']['native_session']['last_outcome']['typed_result']['word']
assert restored['reopened_word']['native_session']['last_outcome']['typed_result']['word']==states['word_read']['native_session']['last_outcome']['typed_result']['word']
assert states['wrong_case_rejected']['native_session']['last_outcome']['status']=='error'
assert states['wrong_case_rejected']['calculator_memory']['Ans']==states['word_read']['calculator_memory']['Ans']
assert restored['reopened_value']['native_session']['last_outcome']['result']=='42'
assert restored['reopened_formula']['native_session']['last_outcome']['result']=='9'
assert created['native_tools']['memory']['models']['spreadsheet']==reopened['native_tools']['memory']['models']['spreadsheet']
print('Qualified worksheet: Home/PPL/live context/EXPR/CAS,typed definitions/words/BCD,INPUT and CHOOSE cancel/confirm,case rejection and separate-process restoration passed')
