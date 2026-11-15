#!/usr/bin/env python3
"""On-screen SIN/Delete/Enter buttons and immediate keyboard entry, both modes."""
import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
data = json.loads((root / 'native_side_sin_pointer.json').read_text())
assert data['wait_timeouts'] == [], data['wait_timeouts']
states = {item['label']: item['state']['native_session'] for item in data['snapshots']}
for mode in ['textbook', 'algebraic']:
    clicked = states[mode + '_clicked']
    assert clicked['presentation']['entry'] == mode
    assert clicked['home_input'] == 'SIN()' and clicked['cursor'] == 4
    assert states[mode + '_deleted']['home_input'] == ''
    assert states[mode + '_typed_directly']['home_input'] == 'SIN(0)'
    assert states[mode + '_result']['last_outcome']['result'] == '0'
    typed = states[mode + '_typed_open']
    assert typed['home_input'] == 'sin()' and typed['cursor'] == 4
    assert states[mode + '_typed_deleted']['home_input'] == ''
print('Actual pointer SIN: cursor inside, direct typing, complete empty-call deletion, both entry modes passed')
