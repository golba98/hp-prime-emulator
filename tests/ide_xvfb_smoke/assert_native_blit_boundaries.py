"""Actual IDE Run must match original admission and complete target buffers."""
import json
from pathlib import Path
import sys

out=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
dump=json.loads((out/'native_side_blit_boundaries.json').read_text())
assert not dump['wait_timeouts'],dump['wait_timeouts']
controls=json.loads((Path(__file__).resolve().parents[1]/'fixtures/native-blit-boundary-controls.json').read_text())
states={entry['label']:entry['state'] for entry in dump['snapshots']}


def value(item):
    if item['type']=='list':return [value(v) for v in item['value']]
    assert item['type']=='real',item
    return int(item['value'])


for case in controls['cases']:
    if not case['accepted']:continue
    state=states[case['name']]
    assert not state['worker_alive'],case['name']
    outcome=state['native_session']['last_outcome']
    assert outcome['status']=='success',(case['name'],outcome)
    assert value(outcome['typed_result'])==case['expected'],(case['name'],outcome)
print(f"Native BLIT: {len(states)} unchanged original programs through actual IDE Run match admission, results and every target pixel")
