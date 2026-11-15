#!/usr/bin/env python3
"""Six Native tools, real desktop input/results and a separate process reopen."""
import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')

def states(name):
    data = json.loads((root / f'native_side_{name}.json').read_text())
    assert data['wait_timeouts'] == [], data['wait_timeouts']
    return {item['label']: item['state'] for item in data['snapshots']}

s = states('tools')
for label, state in s.items():
    assert state['native_tools']['error'] is None, (label, state['native_tools']['error'])
    assert all(not entry['error'] for entry in state['native_session']['history']), label

assert s['variable_stored']['calculator_memory']['TOOLVALUE'] == {'type': 'real', 'value': '19', 'origin': {'kind': 'rational', 'numerator': '19', 'denominator': '1', 'negative': False}}
assert s['variable_stored']['calculator_memory']['A'] == {'type': 'real', 'value': '17', 'origin': {'kind': 'rational', 'numerator': '17', 'denominator': '1', 'negative': False}}
assert s['variable_stored']['calculator_memory']['Ans'] == {'type': 'real', 'value': '17', 'origin': {'kind': 'rational', 'numerator': '17', 'denominator': '1', 'negative': False}}
assert s['variable_inserted']['native_session']['home_input'] == 'TOOLVALUE'
assert s['variable_evaluated']['native_session']['history'][-1]['result'] == '19'
assert s['no_match']['native_tools']['page'] == 'Toolbox'
assert s['no_match']['native_tools']['memory']['commands']['selected'] == ''
assert s['no_match']['native_session']['home_input'] == '3+'
assert s['no_match']['native_session']['history'] == s['variable_evaluated']['native_session']['history']
assert s['toolbox_selected']['native_tools']['memory']['commands']['selected'] == 'SIN'
assert s['command_help']['native_tools']['memory']['help_topic'] == 'SIN'
assert s['command_help']['native_session']['home_input'] == '3+'
assert s['command_inserted']['native_session']['home_input'] == '3+SIN()'
assert s['command_evaluated']['native_session']['history'][-1]['result'] == '3'
assert s['template_selected']['native_tools']['memory']['template_selection'] == 'Nth root'
assert s['template_evaluated']['native_session']['history'][-1]['expression'] == 'NTHROOT(3,8)'
assert s['template_evaluated']['native_session']['history'][-1]['result'] == '2'
assert s['units_selected']['native_tools']['memory']['units']['selected'] == 'cm'
assert s['inch_selected']['native_tools']['memory']['units']['selected'] == 'in'
assert s['measurement_inserted']['native_session']['home_input'] == '20_cm+5_in'
assert s['measurement_evaluated']['native_session']['history'][-1]['typed_result'] == {
    'type': 'symbolic', 'value': {'type': 'unit', 'value': '32.7_(cm)'}}
assert s['unit_tool_selected']['native_tools']['memory']['units']['selected'] == 'CONVERT'
conversion = s['unit_conversion']['native_session']['history'][-1]['typed_result']
assert conversion['type'] == 'symbolic' and conversion['value']['type'] == 'unit'
assert conversion['value']['value'].startswith('16.4041994751_') and 'ft' in conversion['value']['value']
assert s['prefix_selected']['native_tools']['memory']['unit_prefix'] == 'k'
assert s['prefix_selected']['native_session']['home_input'] == '2'
assert s['prefix_inserted']['native_session']['home_input'] == '2_km'
assert s['prefix_evaluated']['native_session']['history'][-1]['typed_result'] == {
    'type': 'symbolic', 'value': {'type': 'unit', 'value': '2_km'}}
assert s['memory_home']['native_tools']['page'] == 'Memory'
assert 'TOOLVALUE' not in s['home_deleted']['calculator_memory']
assert s['home_deleted']['calculator_memory']['A'] == {'type': 'real', 'value': '17', 'origin': {'kind': 'rational', 'numerator': '17', 'denominator': '1', 'negative': False}}
assert s['home_deleted']['calculator_memory']['Ans'] == s['prefix_evaluated']['calculator_memory']['Ans']
assert s['cas_stored']['calculator_cas']['bindings']['toolvalue'] == '23'
assert s['memory_cas']['native_tools']['page'] == 'CasVariables'
assert s['cas_inserted']['native_session']['home_input'] == 'toolvalue'
assert s['cas_evaluated']['native_session']['history'][-1]['typed_result'] == {'type': 'integer', 'value': '23'}
assert s['cas_deleted']['calculator_cas']['bindings'] == {}
assert s['cas_deleted']['calculator_cas']['last_value'] == {'type': 'integer', 'value': '23'}
assert s['cas_deleted']['calculator_memory'] == s['home_deleted']['calculator_memory']
assert s['character_selected']['native_tools']['memory']['characters'] == {'page': 7, 'code': 0x3c0}
assert s['character_inserted']['native_session']['home_input'] == 'π'
assert s['character_evaluated']['native_session']['history'][-1]['result'] == '3.14159265359'

r = states('tools_reopen')
assert r['reopened']['calculator_memory']['A'] == {'type': 'real', 'value': '17', 'origin': {'kind': 'rational', 'numerator': '17', 'denominator': '1', 'negative': False}}
assert 'TOOLVALUE' not in r['reopened']['calculator_memory']
assert r['reopened']['calculator_cas']['bindings'] == {}
assert r['reopened']['native_session']['history'] == s['character_evaluated']['native_session']['history']
assert r['restored_toolbox']['native_tools']['memory']['commands'] == s['toolbox_selected']['native_tools']['memory']['commands']
assert r['restored_units']['native_tools']['memory']['units']['query'] == 'metre'
assert r['restored_units']['native_tools']['memory']['unit_prefix'] == 'k'
assert r['restored_memory']['native_tools']['memory']['memory_selection'] == 'CAS variables'
assert r['restored_cas_catalog']['native_tools']['page'] == 'CasVariables'
assert r['restored_templates']['native_tools']['memory']['template_selection'] == 'Fraction'
assert r['restored_template_evaluated']['native_session']['history'][-1]['result'] == '2'
assert r['reopened']['native_tools']['memory']['characters'] == {'page': 7, 'code': 0x3c0}
print('Native six-tool acceptance: insertion, inspection, management, actual math/units and process reopen passed')
