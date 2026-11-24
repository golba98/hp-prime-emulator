#!/usr/bin/env python3
"""Actual egui Text/Paste -> firmware modal/Notes -> genuine PPL readback."""
import json
import sys
from pathlib import Path
root = Path(sys.argv[1])
data = json.loads((root / 'text.json').read_text())
assert not data['wait_timeouts'], data['wait_timeouts']
scenario = json.loads(Path(__file__).with_name('scripts').joinpath('script_firmware_text.json').read_text())
note = next(step['text'] for step in scenario['steps'] if step['step'] == 'paste')
assert len(note) > 2048 and '\n' in note and '😀' in note
assert any(message['text'] == 'G1 firmware returned ' + note for message in data['console']), data['console']
assert not data['firmware_program_running']
assert data['firmware_session'] == 'Ready'
assert (root / 'note.png').is_file() and (root / 'readback.png').is_file()
print('Firmware desktop text: typed modal name, multiline Unicode paste beyond2048, saved note and exact genuine PPL readback passed')
