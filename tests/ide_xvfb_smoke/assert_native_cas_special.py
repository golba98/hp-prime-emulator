#!/usr/bin/env python3
"""Actual keyboard special-value execution, failed transaction and GUI reopen."""
import json
import sys
from pathlib import Path
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
def states(name):
 data=json.loads((root/f'native_side_{name}.json').read_text())
 assert not data['wait_timeouts'],data['wait_timeouts']
 return {v['label']:v['state'] for v in data['snapshots']}
s=states('cas_special')
assert s['home_undefined']['calculator_memory']['special']=={'type':'symbolic','value':{'type':'undefined'}}
assert s['home_undefined_type']['native_session']['history'][-1]['result']=='8'
assert s['home_infinity']['native_session']['history'][-1]['result']=='+Inf'
assert s['home_eval_string']['native_session']['history'][-1]['typed_result']=={'type':'string','value':'1+2'}
assert s['stored_string']['calculator_cas']['bindings']['saved']=='"1+2"'
assert s['string_type']['native_session']['history'][-1]['result']=='2'
assert s['exact_list_data']['native_session']['history'][-1]['result']=='{1/4,"1/3"}'
assert s['cas_undefined']['calculator_cas']['bindings']['persisted']=='undef'
assert s['cas_negative']['native_session']['history'][-1]['result']=='-Inf'
assert s['failed_syntax']['native_session']['history'][-1]['error'] is True
assert 'Invalid CAS expression' in s['failed_syntax']['native_session']['history'][-1]['result']
assert s['transaction_preserved']['calculator_cas']['bindings']==s['cas_negative']['calculator_cas']['bindings']
assert s['transaction_preserved']['calculator_cas']['last_value']==s['cas_negative']['calculator_cas']['last_value']
r=states('cas_special_reopen')
assert r['restored']['calculator_memory']['special']==s['home_undefined']['calculator_memory']['special']
assert r['home_undefined_readback']['native_session']['history'][-1]['result']=='undef'
assert r['cas_undefined_readback']['native_session']['history'][-1]['result']=='undef'
assert r['cas_negative_readback']['native_session']['history'][-1]['result']=='-Inf'
print('Native CAS: genuine-value TYPE/STRING, undefined/signed infinity, failed assignment rollback and process reopen passed')
