#!/usr/bin/env python3
"""Installed Help navigation and a separate-process profile restoration."""
import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')

def states(name):
    data = json.loads((root / f'native_side_{name}.json').read_text())
    assert not data['wait_timeouts'], data['wait_timeouts']
    return {item['label']: item['state'] for item in data['snapshots']}

opened = states('help_installed')
reopened = states('help_installed_reopen')['reopened']
for label, state in [*opened.items(), ('reopened', reopened)]:
    tool = state['native_tools']
    assert tool['page'] == 'Help', (label, tool['page'])
    assert tool['error'] is None, (label, tool['error'])
    current = tool['memory']['help']['current']
    assert current['language'] == 1, (label, current)
    assert isinstance(current['id'], int) and current['id'] > 0, (label, current)
    assert not state['native_session']['home_input'], label
    assert not state['native_session']['history'], label

for label, name in [('sin', 'SIN'), ('cos', 'COS'), ('back', 'SIN'), ('forward', 'COS'), ('scrolled', 'COS')]:
    memory = opened[label]['native_tools']['memory']
    assert memory['help_topic'] == name, (label, memory['help_topic'])
    assert memory['help']['current']['name'] == name, (label, memory['help']['current'])

sin = opened['sin']['native_tools']['memory']['help']['current']
cos = opened['cos']['native_tools']['memory']['help']['current']
assert sin['id'] != cos['id']
assert opened['back']['native_tools']['memory']['help']['current'] == sin
assert opened['forward']['native_tools']['memory']['help']['current'] == cos
assert reopened['native_tools']['memory']['help']['current'] == cos
assert reopened['native_tools']['memory']['help_query'] == 'COS'
assert reopened['native_tools']['memory']['help']['back'] == opened['scrolled']['native_tools']['memory']['help']['back']
assert opened['scrolled']['native_tools']['help']['scroll_y'] > opened['cos']['native_tools']['help']['scroll_y']
assert opened['sin']['native_tools']['help']['installed']
assert opened['sin']['native_tools']['help']['rendered_math'] > 0
assert not opened['sin']['native_tools']['help']['loading']
assert reopened['native_tools']['help']['installed']
print('Installed Help: real topic selection, keyboard history and separate-process restoration passed.')
