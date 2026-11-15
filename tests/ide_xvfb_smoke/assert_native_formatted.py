#!/usr/bin/env python3
"""Actual desktop formatted values, history controls and separate-process reopen."""
import json
from pathlib import Path

root = Path('/tmp/ppl-ide-e2e')
checks = 0


def check(condition, detail):
    global checks
    assert condition, detail
    checks += 1


def states(name):
    data = json.loads((root / f'native_side_{name}.json').read_text())
    check(data['wait_timeouts'] == [], data['wait_timeouts'])
    return {snapshot['label']: snapshot['state'] for snapshot in data['snapshots']}


def last(state):
    return state['native_session']['history'][-1]


created = states('formatted')
for label, result, mode in [
    ('dms_typed', '1°22′30″', 'sexagesimal'),
    ('dms_decimal', '1.375', 'decimal'),
    ('dms_restored', '1°22′30″', 'sexagesimal'),
]:
    value = last(created[label])
    check(value['result'] == result, (label, value))
    check(value['presentation'] == mode, (label, value))
    check(value['typed_result'] == {'type': 'real', 'value': '1.375', 'presentation': 'sexagesimal', 'origin': {'kind': 'rational', 'numerator': '11', 'denominator': '8', 'negative': False}, 'recall': '1.375'}, (label, value))
    check(created[label]['calculator_memory']['A'] == {k: v for k, v in value['typed_result'].items() if k != 'recall'}, (label, 'stored value altered by presentation'))

check(last(created['dms_collection'])['typed_result']['value'][1]['presentation'] == 'sexagesimal', 'list cell format')
check(last(created['polar_typed'])['result'] == '2∡90', 'polar entry automatically retains its format')
check(last(created['polar_rectangular'])['result'] == '2*i', 'rectangular toggle')
check(last(created['polar_rectangular'])['presentation'] == 'decimal', 'history mode is explicit')
check(created['polar_rectangular']['calculator_memory']['B']['presentation'] == 'polar', 'toggle preserves stored polar value')

reopened = states('formatted_reopen')
restored = reopened['restored']
check(restored['calculator_memory']['A']['presentation'] == 'sexagesimal', 'real metadata restored')
check(restored['calculator_memory']['L1']['value'][1]['presentation'] == 'sexagesimal', 'collection metadata restored')
check(restored['calculator_memory']['B']['presentation'] == 'polar', 'complex metadata restored')
check(last(restored)['presentation'] == 'decimal', 'selected history presentation restored')
check(last(restored)['result'] == '2*i', 'selected display restored')
check(last(reopened['restored_polar_toggle'])['result'] == '2∡90', 'toggle resumes correctly after reopening')
check(last(reopened['dms_after_restart'])['result'] == '2°45′00″', 'real arithmetic after restart')
check(last(reopened['collection_after_restart'])['result'] == '2°00′00″', 'collection extraction after restart')
check(last(reopened['polar_after_restart'])['result'] == '4∡90', 'complex arithmetic after restart with retained angle mode')
check(last(reopened['hms_command'])['result'] == '54°52′34.68″', 'arrow-prefix HMS command')
check(last(reopened['hms_command'])['typed_result']['value'] == '54.8763', 'HMS preserves its numeric value')
check(last(reopened['hms_from_command'])['result'] == '54.8763', 'arrow-suffix HMS conversion')
check(last(reopened['hms_from_command'])['typed_result'].get('presentation') is None, 'HMS clears the representation flag')
print(f'Native formatted values: {checks} desktop assertions passed')
