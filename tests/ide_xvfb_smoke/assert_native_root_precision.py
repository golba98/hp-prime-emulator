"""Home and unchanged original PPL distinguish exact from decimal roots."""
import json
from pathlib import Path
import sys

out=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
dump=json.loads((out/'native_side_root_precision.json').read_text())
assert not dump['wait_timeouts'],dump['wait_timeouts']
states={entry['label']:entry['state'] for entry in dump['snapshots']}
for label,expected in [('rational','1.24721912892'),('decimal','1.24721912893'),('half_power','1.24721912892'),('inverse_root','0.801783725737'),('perfect','2'),('pi','0.669924585691')]:
    state=states[label]
    assert state['calculator_numeric_version']==15751,label
    assert not state['worker_alive'],label
    result=state['native_session']['last_outcome']
    assert result['status']=='success',(label,result)
    assert result['typed_result']['value']==expected,(label,result)
    if label!='perfect':assert result['typed_result']['origin']=={'kind':'decimal'},(label,result)
controls=json.loads((Path(__file__).resolve().parents[1]/'fixtures/native-root-precision-controls.json').read_text())
result=states['program']['native_session']['last_outcome']
assert result['status']=='success',result
assert result['typed_result']['type']=='string',result
assert result['typed_result']['value']==controls['expected'],result
for case in controls['additional_cases']:
    result=states['program_'+case['entry']]['native_session']['last_outcome']
    assert result['status']=='success',result
    assert result['typed_result']['type']=='string',result
    assert result['typed_result']['value']==case['expected'],result
print('Native roots: six actual Home calculations and two unchanged original45value PPL programs match exact/decimal/power/π results')
