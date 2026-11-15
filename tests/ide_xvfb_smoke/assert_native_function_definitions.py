"""Actual definition assignment, typed getters, editing and separate-process restore."""
import json
import sys
from pathlib import Path
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
created=json.loads((root/'native_side_function_definitions.json').read_text())
reopened=json.loads((root/'native_side_function_definitions_reopen.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states={s['label']:s['state'] for s in created['snapshots']}
restored={s['label']:s['state'] for s in reopened['snapshots']}
def graph(label):return states[label]['native_tools']['memory']['graphs']['Function']
def curves(label):return graph(label)['curves']
def result(label):return states[label]['native_session']['last_outcome']
for label,state in list(states.items())+list(restored.items()):
    assert state['native_tools']['error'] is None,(label,state['native_tools']['error'])
    assert not state['worker_alive'] and not state['native_tools']['calculating'],label
    assert not any(name in state['calculator_memory'] for name in ['F1','F2','F0']),label
    assert not any(name in state['calculator_programs'] for name in ['F1','F2','F0']),label
    if state['native_tools']['page'] is not None:
        assert state['native_session']['home_input']=='12+',label
assert result('ppl_definitions')['result']=='{8,"X+0",X+0,2,8,"F1(X)+2",4,0,42,7,99}'
assert curves('symbolic')[0]['expression']=='X+0' and curves('symbolic')[0]['enabled']
assert curves('symbolic')[1]['expression']=='F1(X)+2' and not curves('symbolic')[1]['enabled']
assert curves('symbolic')[9]['expression']=='42' and not curves('symbolic')[9]['enabled']
assert states['numeric']['native_tools']['table'][:2]==[[0,0],[0.1,0.1]]
assert result('atomic')['result']=='{1,1,"X+0",3,"",2}'
assert curves('cleared')[1]['expression']=='' and not curves('cleared')[1]['enabled']
word={'kind':'integer','base':16,'bits':8,'signed_word':True,'value':'255'}
for label in ['word','final_word']:
    value=result(label)['typed_result']['value']
    assert value[0]['value']=='1'
    assert value[1]['word']==value[2]['word']==word
    assert all(v['word']==word for v in value[3]['value'])
    assert curves(label)[9]['literal'][1]==word
assert not curves('word_symbolic')[9]['enabled']
assert curves('edited')[9]['expression']=='X+5' and curves('edited')[9]['enabled']
assert 'literal' not in curves('edited')[9]
assert states['edited_numeric']['native_tools']['table'][:2]==[[0,0,5],[0.1,0.1,5.1]]
assert curves('final')[9]['enabled'] and curves('final')[9]['literal'][1]==word
assert states['word_numeric']['native_numeric_values'][0][2]['word']==word
assert states['word_numeric']['native_tools']['table'][:2]==[[0,0,255],[0.1,0.1,255]]
for label,state in restored.items():
    assert state['native_tools']['memory']['graphs']['Function']==graph('final'),label
assert restored['reopened_values']['native_session']['last_outcome']['result']=='{8,"X+0",2,1,#FF:-8h,#FF:-8h,99,7}'
assert restored['reopened_final']['native_session']['home_input']=='12+'
print('Function definitions: actual PPL/string/quoted/scalar assignment, syntax-preserving getters, checked states, atomic rejection/clear, typed words, UI edit invalidation, Numeric values and separate-process restore passed')
