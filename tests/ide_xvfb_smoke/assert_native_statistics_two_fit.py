"""Check live PPL Statistics 2Var definitions, calculated fits and reopening."""
import json
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def load(suffix=''):
    report = json.loads((root / f'native_side_statistics_two_fit{suffix}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {item['label']: item['state'] for item in report['snapshots']}


def values(state):
    tools = state['native_tools']
    assert tools['error'] is None and not tools['calculating'], tools.get('error')
    result = state['native_session']['last_outcome']
    assert result['status'] == 'success', result
    typed = result['typed_result']
    assert typed['type'] == 'list', typed
    return typed['value']


def real(entry):
    assert entry['type'] == 'real', entry
    return float(entry['value'])


def expression(entry):
    assert entry['type'] == 'symbolic', entry
    assert entry['value']['type'] == 'symbolic', entry
    return entry['value']['value']


def book(state):
    return state['native_tools']['memory']['models']['statistics2_book']


main = load()
factory = values(main['factory_record'])
assert [real(entry) for entry in factory[:9]] == [6, 8, 8, 2, 0, 2, 1, 0, 1]
assert all(entry['type'] == 'list' for entry in factory[9:])
s1, s2 = [entry['value'] for entry in factory[9:]]
assert [expression(entry) for entry in s1[:2]] == ['C1', 'C2']
assert [entry['value'] for entry in s2[:3]] == ['', '', '']
assert s1[4]['value'] == s2[4]['value'] == ''
for record, index, color in [(s1, 1, 0x0000ff), (s2, 2, 0xff0000)]:
    assert len(record) == 8 and real(record[3]) == 1 and real(record[6]) == index
    for field in [5, 7]:
        style = record[field]
        assert style['type'] == 'integer' and style['word']['bits'] == 24, style
        assert int(style['word']['value']) == color, style

a, kind, fit, evaluated = values(main['first_fit'])
assert [real(entry) for entry in [a, kind, evaluated]] == [1, 8, 9]
assert expression(fit) == book(main['first_fit'])['fit_equations'][0]

stale, a, refitted, evaluated = values(main['retained_then_refitted'])
assert [real(entry) for entry in [stale, a, evaluated]] == [9, 1, 13]
assert expression(refitted) == book(main['retained_then_refitted'])['fit_equations'][0]
assert [real(entry) for entry in values(main['failed_keeps_fit'])] == [1, 13]
assert book(main['failed_keeps_fit'])['fit_equations'][0] == expression(refitted)

kind, new, evaluated = values(main['plot_refreshed_fit'])
assert [real(kind), real(evaluated)] == [8, 17]
assert expression(new) == book(main['plot_refreshed_fit'])['fit_equations'][0]
assert main['plot_refreshed_fit']['native_tools']['page'] == 'Plot'
reopened = load('_reopen')['reopened']
kind, restored, evaluated, selected = values(reopened)
assert [real(kind), real(evaluated), real(selected)] == [8, 17, 1]
assert expression(restored) == expression(new)
assert book(reopened)['fit_equations'] == book(main['plot_refreshed_fit'])['fit_equations']
assert reopened['native_tools']['page'] == 'Symbolic'

for name in ['symbolic', 'reopened']:
    with Image.open(root / f'statistics_two_fit_{name}.png') as image:
        assert image.size == (320, 240)
        fit_row = image.crop((44, 77, 170, 96)).convert('L')
        assert sum(pixel < 96 for pixel in fit_row.tobytes()) > 28
print('Statistics 2Var typed S1–S5 records, retained fits, plot refresh and process reopen passed')
