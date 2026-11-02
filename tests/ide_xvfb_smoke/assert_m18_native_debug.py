#!/usr/bin/env python3
"""Assert real linked debugger states from an isolated native-only IDE window."""
import json
import sys
from pathlib import Path
report = json.loads(Path(sys.argv[1]).read_text())
snapshots = {entry['label']: entry['state'] for entry in report['snapshots']}
checks = 0
assert not report['wait_timeouts'], report['wait_timeouts']
checks += 1
for label, index, line in [('root_call',0,5),('middle',1,3),('middle_call',1,4),('leaf_breakpoint',2,5),('middle_return',1,5),('root_return',0,6)]:
    state = snapshots[label]
    assert state['vm_state'] == 'Paused', state['vm_state']
    assert state['active_tab'] == index, (label,state['active_tab'])
    assert state['debug_pause']['line'] == line, (label,state['debug_pause'])
    assert state['cursor_line'] == line
    checks += 4
leaf = snapshots['leaf_breakpoint']['debug_pause']
assert leaf['depth'] == 3
assert [frame['source'].split('/')[-1].split('.')[0] for frame in leaf['frames']] == ['LEAF','MIDDLE','ROOT']
assert {v['name']:v['value'] for v in leaf['frames'][0]['globals']}['count'] == '10'
assert {v['name']:v['value'] for v in leaf['frames'][2]['globals']}['count'] == '100'
assert {v['name']:v['value'] for v in leaf['frames'][0]['locals']}['x'] == '3'
checks += 5
state = snapshots['caller_frame']
assert state['active_tab'] == 0
assert state['cursor_line'] == 5
checks += 2
state = snapshots['edited_while_paused']
active = state['tabs'][state['active_tab']]
assert active['debug_snapshot']
assert 'RETURN count+x' in active['content']
assert not state['tabs'][2]['debug_snapshot']
assert 'RETURN 42' in state['tabs'][2]['content']
checks += 4
assert snapshots['finished']['worker_outcome']['value'] == '113'
assert snapshots['new_run']['debug_pause'] is not None
assert not snapshots['stopped']['worker_alive']
assert report['window_captures']
checks += 4
print(f'{checks} M18 linked Native debugger GUI assertions passed')
