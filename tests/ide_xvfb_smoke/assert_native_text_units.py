"""Actual GUI execution must preserve raw text, endpoints and full G1 frames."""
import hashlib
import json
from pathlib import Path
import sys
from PIL import Image

out = Path(sys.argv[1] if len(sys.argv) > 1 else '/tmp/ppl-ide-e2e')
dump = json.loads((out / 'native_side_text_units.json').read_text())
assert not dump['wait_timeouts'], dump['wait_timeouts']
controls = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-text-unit-controls.json').read_text())
states = {item['label']: item['state'] for item in dump['snapshots']}


def value(typed):
    if typed['type'] == 'list':
        return [value(item) for item in typed['value']]
    if typed['type'] == 'real':
        return int(typed['value'])
    assert typed['type'] == 'string', typed
    return typed['value']


for case in controls['cases']:
    name = case['name']
    outcome = states[name]['native_session']['last_outcome']
    assert outcome['status'] == 'success', (name, outcome)
    assert value(outcome['typed_result']) == case['expected'], (name, outcome)
    image = Image.open(out / f'native_text_{name}.png').convert('RGB')
    assert image.size == (320, 240), (name, image.size)
    assert hashlib.sha256(image.tobytes()).hexdigest() == case['rgb_sha256'], name
print(f"Native text: {len(controls['cases'])} actual IDE programs match every original return value and complete G1 frame")
