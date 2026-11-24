#!/usr/bin/env python3
"""Actual IDE Run, calculator Enter and complete original-G1 menu frames."""
import hashlib
import json
from pathlib import Path
import sys
from PIL import Image

out = Path(sys.argv[1] if len(sys.argv) > 1 else '/tmp/ppl-ide-e2e')
dump = json.loads((out / 'native_side_menus.json').read_text())
assert not dump['wait_timeouts'], dump['wait_timeouts']
controls = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-menu-controls.json').read_text())
snapshots = {item['label']: item['state'] for item in dump['snapshots']}
for case in controls['cases']:
    name = case['name']
    outcome = snapshots[name]['native_session']['last_outcome']
    assert outcome['status'] == 'success', (name, outcome)
    assert outcome['result'].replace(' ', '') == '{1,0}', (name, outcome)
    image = Image.open(out / f'native_menu_{name}.png').convert('RGB')
    assert image.size == (320, 240), (name, image.size)
    assert hashlib.sha256(image.tobytes()).hexdigest() == case['rgb_sha256'], name
print(f"Native menus: {len(controls['cases'])} actual IDE programs and calculator Enter interactions match complete original G1 frames")
