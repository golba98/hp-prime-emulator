"""PPL checkbox commands and the actual Symbolic controls share two saved books."""
import json
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def load(suffix=''):
    report = json.loads((root / f'native_side_statistics_check{suffix}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {item['label']: item['state'] for item in report['snapshots']}


def result(state, expected):
    tools = state['native_tools']
    assert tools['page'] == 'Symbolic' and tools['error'] is None
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    typed = outcome['typed_result']
    assert typed['type'] == 'list'
    assert all(item['type'] == 'real' for item in typed['value'])
    assert [int(item['value']) for item in typed['value']] == expected, typed


def checkbox(state, which, expected):
    book = state['native_tools']['memory']['models'][f'statistics{which}_book']
    assert [analysis['enabled'] for analysis in book['analyses']] == expected
    return book


main = load()
result(main['one_changed'], [1, 1, 1, 1, 0, 1, 0])
one = checkbox(main['one_changed'], 1, [False, True, False, False, False])
assert one['analyses'][1]['data_source'] == 'D2'
result(main['two_changed'], [1, 1, 1, 1, 0, 1])
checkbox(main['two_changed'], 2, [True, False, False, False, False])
checkbox(main['two_pointer_off'], 2, [False] * 5)
result(main['two_pointer_read'], [0, 0, 0])
result(main['one_still_checked'], [0, 1, 0])

reopened = load('_reopen')
result(reopened['two_reopened'], [0, 0, 0])
checkbox(reopened['two_reopened'], 2, [False] * 5)
result(reopened['one_reopened'], [0, 1, 0])
restored = checkbox(reopened['one_reopened'], 1, [False, True, False, False, False])
assert restored == one


def screen_checkbox(label):
    with Image.open(root / f'statistics_check_{label}.png') as image:
        assert image.size == (320, 240)
        return image.crop((1, 24, 15, 39)).convert('RGB').tobytes()


assert screen_checkbox('two_changed') != screen_checkbox('two_pointer_off')
print('Statistics CHECK, UNCHECK and ISCHECK share real Symbolic selections and preserve both app states after reopen')
