"""Geometry command captions through real plot rendering and PPL settings."""
import json
import math
from pathlib import Path
import sys

from PIL import Image, ImageChops

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
COMMAND_BACKGROUND = (224, 237, 243)


def snapshots(suffix='', root=ROOT):
    path = root / f'native_side_geometry_scroll_text{suffix}.json'
    report = json.loads(path.read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {item['label']: item['state'] for item in report['snapshots']}


def result(state):
    value = state['native_session']['last_outcome']
    assert value['status'] == 'success', value
    assert value['typed_result']['type'] == 'list', value
    typed = value['typed_result']['value']
    assert all(item['type'] == 'real' for item in typed), typed
    return [float(item['value']) for item in typed]


def crop(label, state, box=(126, 198, 320, 219), root=ROOT):
    (left, top), (right, bottom) = state['lcd_rect']['min'], state['lcd_rect']['max']
    x0, y0, x1, y1 = box
    bounds = (
        left + x0 * (right - left) / 320, top + y0 * (bottom - top) / 240,
        left + x1 * (right - left) / 320, top + y1 * (bottom - top) / 240,
    )
    path = root / f'geometry_scroll_text_{label}.png'
    with Image.open(path).convert('RGB') as image:
        return image.crop(tuple(map(round, bounds)))


def same(a, b):
    return ImageChops.difference(a, b).getbbox() is None


def pointer(state):
    return state['native_tools']['memory']['geometry_cursor']['pointer']


def definitions(state):
    memory = state['native_tools']['memory']
    return [entry['source'] for entry in memory['models']['geometry_book']['objects']]


states = snapshots()
reopened = snapshots('_reopen')
for label, state in [*states.items(), *reopened.items()]:
    tools = state['native_tools']
    expected = 'PlotSetup' if label.startswith('setup_') else 'Plot'
    assert tools['page'] == expected and not tools['calculating'] and tools['error'] is None, label

assert result(states['idle_manual']) == result(states['read_manual']) == [0, 0]
assert result(states['typed_automatic']) == [0, 1, 1, 1, 0, 1]
assert result(states['saved_automatic']) == result(reopened['reopened_flag']) == [0, 1]
assert crop('idle_manual', states['idle_manual']).getpixel((1, 1)) != COMMAND_BACKGROUND
head = crop('manual_head', states['manual_head'])
tail = crop('manual_tail', states['manual_tail'])
assert head.getpixel((1, 1)) == tail.getpixel((1, 1)) == COMMAND_BACKGROUND
assert same(head, crop('manual_idle', states['manual_idle']))
assert not same(head, tail)
assert same(tail, crop('manual_tail_bound', states['manual_tail_bound']))
assert same(tail, crop('manual_tail_idle', states['manual_tail_idle']))
assert same(head, crop('manual_returned', states['manual_returned']))

for label in ('manual_idle', 'manual_tail', 'manual_tail_bound', 'manual_tail_idle', 'manual_returned'):
    assert pointer(states[label]) == pointer(states['manual_head'])
    assert definitions(states[label]) == ['point(2,1)']
old = pointer(states['manual_head'])
new = pointer(states['keyboard_adjusted'])
assert math.isclose(new[0] - old[0], 0.025123456789, abs_tol=1e-10) and new[1] == old[1]
adjusted = crop('keyboard_adjusted', states['keyboard_adjusted'])
prefix = (0, 0, head.width // 2, head.height)
assert same(head.crop(prefix), adjusted.crop(prefix))
assert not same(head, adjusted)
assert definitions(states['staged_operand']) == ['point(2,1)', 'point(4.44333333864,0.443024802348)']
assert definitions(states['confirmed'])[-1] == 'perpen_bisector(GB,GC)'

assert same(crop('auto_short', states['auto_short']), crop('auto_short_later', states['auto_short_later']))
for a, b in (('auto_early', 'auto_later'), ('auto_later', 'auto_latest')):
    assert not same(crop(a, states[a]), crop(b, states[b])), (a, b)
    assert pointer(states[a]) == pointer(states[b])
assert not same(
    crop('setup_automatic', states['setup_automatic'], (8, 201, 312, 219)),
    crop('setup_manual', states['setup_manual'], (8, 201, 312, 219)),
)
assert not same(
    crop('reopened_early', reopened['reopened_early']),
    crop('reopened_later', reopened['reopened_later']),
)

if len(sys.argv) > 2:
    baseline = Path(sys.argv[2])
    before = snapshots(root=baseline)
    assert result(before['idle_manual']) == [0, 0]
    assert crop('manual_head', before['manual_head'], root=baseline).getpixel((1, 1)) != COMMAND_BACKGROUND

print('Geometry ScrollText passed for PPL, manual drag, automatic scrolling, setup and restart')
