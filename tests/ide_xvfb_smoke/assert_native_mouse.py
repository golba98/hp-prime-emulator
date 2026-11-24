"""Actual native PPL reads pointer packets delivered by real LCD mouse events."""
import json
import sys
from pathlib import Path
root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
data = json.loads((root/'native_side_mouse.json').read_text())
assert not data['wait_timeouts'], data['wait_timeouts']
states = {s['label']: s['state'] for s in data['snapshots']}

def result(label):
    state = states[label]
    assert not state['worker_alive'] and not state['native_tools']['calculating'], label
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', (label, outcome)
    return outcome['typed_result']

def items(value):
    assert value['type'] == 'list', value
    return value['value']

def idle(value):
    assert [items(p) for p in items(value)] == [[], []], value

def contact(value, expected):
    pointers = items(value)
    assert len(pointers) == 2 and items(pointers[1]) == []
    packet = items(pointers[0])
    assert len(packet) == 5
    for index, (cell, number) in enumerate(zip(packet, expected)):
        assert cell['type'] == 'integer'
        word = cell['word']
        assert word['kind'] == 'integer' and int(word['value']) == number, (index, word)
        assert word['bits'] == 16 and word['system_base'] is True, (index, word)
        assert word.get('signed_word', False) == (index < 4), (index, word)
        assert not word.get('negative', False), (index, word)

observed = items(result('drag'))
assert len(observed) == 4
idle(observed[0])
contact(observed[1], [100, 50, 100, 50, 0])
contact(observed[2], [120, 70, 100, 50, 2])
idle(observed[3])
idle(result('idle'))
second = items(result('new_contact'))
contact(second[0], [30, 40, 30, 40, 0])
idle(second[1])
print('Native MOUSE: real LCD drag and second tap, original coordinates, New/Drag classification, typed SYSTEM words and two empty pointer lists after release passed')
