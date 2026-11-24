"""Intrinsic word sign/width/base, selected-history editing and process restore."""
import json
import sys
from pathlib import Path
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
def load(name):
    d=json.loads((root/f'native_side_{name}.json').read_text());assert not d['wait_timeouts'],d['wait_timeouts']
    return {i['label']:i['state'] for i in d['snapshots']}
s=load('word_metadata');r=load('word_metadata_reopen')
def word(state):return state['native_session']['history'][-1]['typed_result']['word']
def edit(state):return state['native_tools']['integer_edit']
def flags(w):return w['bits'],w.get('signed_word',False),w.get('negative',False),w.get('system_base',False)
assert flags(word(s['positive_word']))==(8,True,False,False)
assert word(s['positive_word'])['value']=='255'
e=edit(s['positive_editor']);assert (e['raw'],e['bits'],e['signed'],e['negative'])==(255,8,True,False)
e=edit(s['negated_copy']);assert (e['raw'],e['bits'],e['signed'],e['negative'])==(1,8,True,True)
assert edit(s['cached_copy'])==e
for label in ['positive_editor','negated_copy','saved_copy','cached_copy']:
    assert s[label]['native_session']['ans']==s['positive_word']['native_session']['ans'],label
assert flags(word(s['negative_word']))==(8,True,True,False)
assert word(s['negative_word'])['value']=='251'
e=edit(s['negative_editor']);assert e['raw']==251 and e['negative'] and e['signed']
original=word(s['system_word']);assert flags(original)==(16,False,False,True) and original['value']=='4123'
selection=s['selected_system_history']['native_session'];assert selection['history_position']==2 and selection['history_result']
e=edit(s['selected_system_editor']);assert (e['raw'],e['bits'],e['base'],e['system_base'])==(4123,16,10,True)
assert s['selected_system_editor']['native_session']['ans']==s['changed_settings']['native_session']['ans']
e=edit(s['explicit_editor']);assert e['base']==16 and not e['system_base']
source=s['history_copy']['native_session']['home_input'];assert source=='Setbase(#101B:16h,0)',source
assert word(s['recalled_word'])==original
assert r['restored']['native_session']['ans']==s['recalled_word']['native_session']['ans']
e=edit(r['restored_system_editor']);assert (e['raw'],e['bits'],e['base'],e['system_base'])==(4123,16,10,True)
assert r['oldest_selected']['native_session']['history_position']==0
assert r['oldest_selected']['native_session']['history_result']
e=edit(r['restored_cached_copy']);assert (e['raw'],e['bits'],e['signed'],e['negative'])==(1,8,True,True)
assert r['restored_cached_copy']['native_session']['ans']==r['restored']['native_session']['ans']
print('Word GUI: intrinsic signed carrier,copy isolation,selected history,current system base,exact recall and separate-process saved copies passed')
