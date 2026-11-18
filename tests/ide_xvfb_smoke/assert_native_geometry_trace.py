"""Geometry point tracing, metadata and fresh-process rendering."""
import json
import math
from pathlib import Path
import sys

from PIL import Image

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def snapshots(suffix=''):
    path = ROOT / f'native_side_geometry_trace{suffix}.json'
    report = json.loads(path.read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {item['label']: item['state'] for item in report['snapshots']}


def object_state(state):
    book = state['native_tools']['memory']['models']['geometry_book']
    assert 'traces' not in book
    point, = book['objects']
    return point


def result(state, flag):
    value = state['native_session']['last_outcome']
    assert value['status'] == 'success', value
    assert value['typed_result']['type'] == 'list', value
    typed = value['typed_result']['value']
    assert [item['type'] for item in typed] == ['real'] * 5 + ['string'], typed
    assert [float(item['value']) for item in typed[:3]] == [flag, flag, 1], typed
    assert (' t(1)' in typed[5]['value']) == bool(flag)
    return [float(item['value']) for item in typed[3:5]]


def trail(label, state):
    (left, top), (right, bottom) = state['lcd_rect']['min'], state['lcd_rect']['max']
    colors = set()
    with Image.open(ROOT / f'geometry_trace_{label}.png').convert('RGB') as img:
        for y in range(219):
            sy = math.floor(top + (y + 0.5) * (bottom - top) / 240)
            for x in range(320):
                sx = math.floor(left + (x + 0.5) * (right - left) / 320)
                if img.getpixel((sx, sy)) == (23, 122, 153):
                    colors.add((x, y))
    return colors


states = snapshots()
reopened = snapshots('_reopen')
for label, state in [*states.items(), *reopened.items()]:
    tools = state['native_tools']
    assert tools['page'] == 'Plot' and not tools['calculating'] and tools['error'] is None, label

assert not object_state(states['prepared']).get('traced', False)
assert not trail('prepared', states['prepared'])
dotted = trail('dotted', states['dotted'])
assert 4 <= len(dotted) < 40, dotted
assert all(169 <= x <= 213 and 88 <= y <= 136 for x, y in dotted), dotted
connected = trail('connected', states['connected'])
assert len(connected) >= len(dotted) * 2
assert dotted <= connected
assert len(trail('view_return', states['view_return']) & connected) >= len(connected) - 1
assert result(states['typed_enabled'], 1) == [5.2, -2.6]

stopped = trail('stopped', states['stopped'])
assert connected <= stopped, (connected, stopped)
assert all(x <= 213 for x, _ in stopped), stopped
assert result(states['typed_stopped'], 0) == [9., 3.4]
assert not object_state(states['cleared']).get('traced', False)
assert not object_state(states['cleared']).get('trace_samples')
assert not trail('cleared', states['cleared'])

assert len(trail('saved_armed', states['saved_armed'])) >= 4
assert result(states['saved_armed'], 1) == result(reopened['reopened'], 1) == [5.8, 5.]
assert object_state(reopened['reopened']) == object_state(states['saved_armed'])
assert object_state(reopened['reopened'])['trace_connected']
saved_trail = trail('saved_armed', states['saved_armed'])
restored_trail = trail('reopened', reopened['reopened'])
assert len(restored_trail & saved_trail) >= len(saved_trail) - 1
fresh_trace = trail('new_trace_after_reopen', reopened['new_trace_after_reopen'])
assert len(fresh_trace & restored_trail) >= len(restored_trail) - 1
assert len(fresh_trace - restored_trail) > 15
print('Geometry Trace passed for dots, connected marks, stop, clear, metadata and saved trails')
