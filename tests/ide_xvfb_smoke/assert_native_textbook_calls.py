"""Verify tall function-call delimiters, typed CAS history, and Show."""
import json
from pathlib import Path
import sys

from PIL import Image


ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
CASES = [
    ('short', 'sin(x)'),
    ('power_argument', 'sin(x^2)'),
    ('sine', 'sin((x+1)/(x^2+2))'),
    ('two_arguments', 'f((x+1)/(y+2),(m+1)/(n+2))'),
    ('nested', 'atan(sin(x/(y+1))/(z+2))'),
]


def snapshots(directory, suffix=''):
    report = json.loads((directory / f'native_side_textbook_calls{suffix}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {entry['label']: entry['state']['native_session'] for entry in report['snapshots']}


def right_delimiter_height(path, crop):
    """Measure the rightmost disconnected mark in the newest expression."""
    with Image.open(path).convert('RGB') as image:
        left, top, right, bottom = crop
        ink = {(x, y) for x in range(left, right) for y in range(top, bottom)
               if max(image.getpixel((x, y))) < 95}
    assert ink, path
    edge = max(x for x, _ in ink)
    candidates = []
    while ink:
        pixels = [ink.pop()]
        for x, y in pixels:
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    adjacent = (x + dx, y + dy)
                    if adjacent in ink:
                        ink.remove(adjacent)
                        pixels.append(adjacent)
        xs, ys = zip(*pixels)
        if max(xs) >= edge - 1:
            candidates.append((max(ys), max(ys) - min(ys) + 1))
    return max(candidates)[1]


def verify_selection(states, history):
    selected, show, returned = (states[label] for label in ('selected', 'show', 'returned'))
    for state in (selected, show, returned):
        assert state['history'] == history
        assert state['history_position'] == len(history) - 1 and state['history_result']
    assert selected['history_viewer'] is None
    assert show['history_viewer'] == {'x': 0, 'y': 0}
    assert returned['history_viewer'] is None


created = snapshots(ROOT)
reopened = snapshots(ROOT, '_reopen')
for count, (label, expression) in enumerate(CASES, start=1):
    history = created[label]['history']
    assert len(history) == count
    entry = history[-1]
    assert not entry['error'] and entry['expression'] == entry['result'] == expression
    assert entry['typed_result'] == {'type': 'symbolic', 'value': expression}
history = created['nested']['history']
assert reopened['reopened']['history'] == history
verify_selection(created, history)
verify_selection(reopened, history)

heights = {
    label: right_delimiter_height(ROOT / f'textbook_call_{label}.png', (160, 40, 320, 201))
    for label, _ in CASES
}
assert 7 <= heights['short'] <= 14, heights
assert heights['power_argument'] >= heights['short'] + 7, heights
assert heights['sine'] >= 32 and heights['two_arguments'] >= 29, heights
assert heights['nested'] >= 45, heights
for label in ['show', 'reopened_show']:
    height = right_delimiter_height(ROOT / f'textbook_call_{label}.png', (0, 22, 318, 213))
    assert height >= 50, (label, height)

if len(sys.argv) == 3:
    legacy = Path(sys.argv[2])
    restored = snapshots(legacy, '_reopen')
    assert restored['reopened']['history'] == history
    verify_selection(restored, history)
    height = right_delimiter_height(legacy / 'textbook_call_reopened_show.png', (0, 22, 318, 213))
    assert height >= 50, height

print('Textbook calls passed for tall arguments, typed History, Show, and reopen')
