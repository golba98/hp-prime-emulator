"""Verify installed Help list placement, actual navigation, and reopen."""
import json
from pathlib import Path
import sys

from PIL import Image


ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def snapshots(suffix=''):
    report = json.loads((ROOT / f'native_side_help_list{suffix}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {s['label']: s['state'] for s in report['snapshots']}


def assert_topic(state, name, topic_id):
    tools = state['native_tools']
    help_state = tools['help']
    assert tools['page'] == 'Help' and tools['error'] is None
    assert help_state['installed'] and not help_state['loading'] and help_state['error'] is None
    assert help_state['topic_id'] == topic_id
    memory = tools['memory']['help']
    assert not memory['contents']
    assert memory['current']['id'] == topic_id and memory['current']['name'] == name
    return help_state['scroll_y']


def marker_centers(label, state):
    """Find compact filled and hollow marker glyphs in the body gutter."""
    (left, top), (right, bottom) = state['lcd_rect']['min'], state['lcd_rect']['max']
    x_scale, y_scale = (right-left)/320, (bottom-top)/240
    with Image.open(ROOT / f'help_lists_{label}.png').convert('RGB') as image:
        ink = set()
        for y in range(round(top+69*y_scale), round(bottom)):
            for x in range(round(left), round(left+25*x_scale)):
                if max(image.getpixel((x, y))) < 70:
                    ink.add((x, y))

    centers = []
    while ink:
        x, y = ink.pop()
        queue = [(x, y)]
        xmin = xmax = x
        ymin = ymax = y
        for x, y in queue:
            xmin, xmax = min(xmin, x), max(xmax, x)
            ymin, ymax = min(ymin, y), max(ymax, y)
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    next_pixel = (x+dx, y+dy)
                    if next_pixel in ink:
                        ink.remove(next_pixel)
                        queue.append(next_pixel)
        width, height = (xmax-xmin+1)/x_scale, (ymax-ymin+1)/y_scale
        if 2.5 <= width <= 6.5 and 2.5 <= height <= 6.5 and .7 <= width/height <= 1.3:
            centers.append(((xmin+xmax)/2-left)/x_scale)
    return centers


states = snapshots()
reopened = snapshots('_reopen')
for label, expected in [('plot_results', 'Function Plot View'), ('home_results', 'Home View')]:
    memory = states[label]['native_tools']['memory']
    assert memory['help']['contents'] and memory['help_query'] == expected

plot, home = 'Function Plot View', 'Home View'
for label in ('plot', 'primary', 'nested', 'back', 'saved_plot'):
    assert_topic(states[label], plot, 82)
for label in ('home', 'forward'):
    assert_topic(states[label], home, 5)
assert 0 < assert_topic(states['primary'], plot, 82) < assert_topic(states['nested'], plot, 82)
assert_topic(reopened['reopened'], plot, 82)
assert assert_topic(reopened['nested_reopened'], plot, 82) > 0

primary = marker_centers('primary', states['primary'])
assert sum(2 < position < 9 for position in primary) >= 3, primary
for label, state in [('nested', states['nested']), ('nested_reopened', reopened['nested_reopened'])]:
    nested = marker_centers(label, state)
    assert sum(14 < position < 23 for position in nested) >= 3, (label, nested)
print('Installed Help lists passed for primary and nested marker columns, navigation, and reopen')
