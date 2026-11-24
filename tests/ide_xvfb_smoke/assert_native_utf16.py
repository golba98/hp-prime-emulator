#!/usr/bin/env python3
"""Actual Native keyboard execution and process reopen with binary strings."""
import json
import sys
from pathlib import Path
root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
for name in ['utf16', 'utf16_reopen']:
    data = json.loads((root / f'native_side_{name}.json').read_text())
    assert not data['wait_timeouts'], data['wait_timeouts']
    states = {item['label']: item['state'] for item in data['snapshots']}
    last = states['unit_count']
    assert last['calculator_memory']['RAW'] == {'type':'string','utf16':[55357,0,56832]}
    assert last['calculator_memory']['RAW2'] == {'type':'string','utf16':[88,0,56832]}
    assert states['raw_readback']['native_session']['history'][-1]['result'] == '{55357,0,56832}'
    assert states['copy_readback']['native_session']['history'][-1]['result'] == '{88,0,56832}'
    assert last['native_session']['history'][-1]['result'] == '3'
    assert all(not row['error'] for row in last['native_session']['history'])
    if name.endswith('reopen'):
        assert states['restored']['calculator_memory']['RAW'] == last['calculator_memory']['RAW']
print('Native UTF16: actual keyboard CHAR/REPLACE/ASC/DIM, independent copies and separate-process memory restoration passed')
