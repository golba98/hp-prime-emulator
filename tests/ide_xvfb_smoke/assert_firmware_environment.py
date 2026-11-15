#!/usr/bin/env python3
"""Assert real desktop menu/slider/radio input and the live helper's reply."""
import json
import sys
from pathlib import Path
path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e/firmware_environment/environment.json')
data = json.loads(path.read_text())
assert not data['wait_timeouts'], data['wait_timeouts']
states = {entry['label']: entry['state'] for entry in data['snapshots']}
before, after = states['initial'], states['changed']
assert before['firmware_environment']['battery'] == {'adc': 1000, 'power': 'battery'}
assert after['firmware_environment']['battery']['power'] == 'charging'
# Pointer at slider midpoint:0.01 V UI resolution plus ADC quantization.
assert abs(after['firmware_environment']['battery']['adc'] * 0.004556000232696533 - 3.825) <= 0.01
assert after['firmware_status'] == 'Virtual calculator environment updated'
assert after['firmware_alive']
assert after['firmware_session'] == 'Ready'
print('Firmware environment:7 desktop interaction/backend acknowledgement assertions passed')
