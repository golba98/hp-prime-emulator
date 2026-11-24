"""Check list braces span their tall items in History and Show."""
import json
from pathlib import Path
import sys

from PIL import Image


def load(root, suffix=''):
    report = json.loads((root / f'native_side_textbook_lists{suffix}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {entry['label']: entry['state']['native_session'] for entry in report['snapshots']}


def result(states, name, expression, count):
    history = states[name]['history']
    assert len(history) == count, history
    entry = history[-1]
    assert entry['expression'] == expression and not entry['error'], entry
    typed = entry['typed_result']
    assert typed['type'] == 'list', typed
    return typed['value']


def check_show(states, name, history):
    for suffix in ('_selected', '_show', '_returned'):
        state = states[name + suffix]
        assert state['history'] == history and not state['home_input']
        assert state['history_position'] == len(history) - 1 and state['history_result']
        viewer = {'x': 0, 'y': 0} if suffix == '_show' else None
        assert state['history_viewer'] == viewer


def brace_heights(root, name):
    """Find the two outside ink components in the latest row or in Show."""
    with Image.open(root / f'textbook_list_{name}.png').convert('RGB') as image:
        ink = {(x, y) for x in range(320) for y in range(22, 202)
               if max(image.getpixel((x, y))) < 95}
    boxes = []
    while ink:
        points = [ink.pop()]
        for x, y in points:
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    neighbor = x + dx, y + dy
                    if neighbor in ink:
                        ink.remove(neighbor)
                        points.append(neighbor)
        xs, ys = zip(*points)
        boxes.append((min(xs), max(xs), min(ys), max(ys)))
    if name.endswith('_show'):
        boundaries = min(boxes, key=lambda box: box[0]), max(boxes, key=lambda box: box[1])
    else:
        boundaries = (
            max((box for box in boxes if box[0] <= 8), key=lambda box: box[3]),
            max((box for box in boxes if box[1] >= 312), key=lambda box: box[3]),
        )
    return tuple(bottom - top + 1 for _, _, top, bottom in boundaries)


root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
main = load(root)
reopened = load(root, '_reopen')
assert [item['value'] for item in result(main, 'home_short', '{1,2}', 1)] == ['1', '2']
assert [item['value'] for item in result(main, 'home_fraction', '{1/2,2/3}', 2)] == [
    '0.5', '0.666666666667',
]
matrix = result(main, 'home_matrix', '{1,[[1,2],[3,4]]}', 3)[1]
assert (matrix['type'], matrix['rows'], matrix['cols']) == ('matrix', 2, 2)
assert [item['value'] for item in matrix['value']] == ['1', '2', '3', '4']
assert result(main, 'cas_short', '{1,2}', 1) == [
    {'type': 'integer', 'value': value} for value in ('1', '2')
]
fractions = result(main, 'cas_fraction', '{1/2,2/3}', 2)
assert fractions == [
    {'type': 'rational', 'value': {'numerator': num, 'denominator': den}}
    for num, den in [('1', '2'), ('2', '3')]
]
nested = result(main, 'cas_nested', '{0,{1/2,sqrt(2/3)}}', 3)
assert nested == [
    {'type': 'integer', 'value': '0'},
    {'type': 'list', 'value': [fractions[0], {'type': 'symbolic', 'value': 'sqrt(6)/3'}]},
]

for name in ['home_matrix', 'cas_fraction', 'cas_nested']:
    check_show(main, name, main[name]['history'])
for name, prior in [('reopened_home', 'home_matrix'), ('reopened_cas', 'cas_nested')]:
    history = main[prior]['history']
    assert reopened[name]['history'] == history
    check_show(reopened, name, history)

for name in ['home_short', 'cas_short']:
    heights = brace_heights(root, name)
    assert all(7 <= height <= 17 for height in heights), (name, heights)
left, right = brace_heights(root, 'home_fraction')
assert left >= 30 and 7 <= right <= 17, (left, right)
for name in [
    'home_matrix', 'home_matrix_show', 'cas_fraction', 'cas_fraction_show', 'cas_nested',
    'cas_nested_show', 'reopened_home', 'reopened_home_show', 'reopened_cas', 'reopened_cas_show',
]:
    heights = brace_heights(root, name)
    assert min(heights) >= 30, (name, heights)

print('Textbook list braces span tall Home and CAS values in History, Show, and reopened profiles')
