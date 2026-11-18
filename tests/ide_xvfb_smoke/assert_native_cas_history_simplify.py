"""Exercise CAS Simplify and equation entry through the actual LCD softkeys."""
import json
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def load(suffix=''):
    report = json.loads((root / f'native_side_cas_history_simplify{suffix}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {item['label']: item['state'] for item in report['snapshots']}


main = load()
sessions = {label: state['native_session'] for label, state in main.items()}


def entry(label, expression, kind, value):
    record = sessions[label]['history'][-1]
    assert not record['error'] and record['expression'] == expression, record
    typed = record['typed_result']
    assert typed == {'type': kind, 'value': value}, typed


assert sessions['empty_cas']['history'] == [] and sessions['empty_cas']['home_input'] == ''
entry('simplified_latest', 'simplify(4)', 'integer', '4')
selected = sessions['selected_history']
assert not selected['history_result']
assert selected['history'][selected['history_position']]['expression'] == '(x^2-1)/(x-1)'
entry('simplified_selected', 'simplify((x^2-1)/(x-1))', 'symbolic', 'x+1')
assert len(sessions['simplified_selected']['history']) == 5
template = sessions['draft_template']
assert template['home_input'] == '2+simplify()' and template['cursor'] == len('2+simplify(')
assert template['history'] == sessions['simplified_selected']['history']
assert sessions['draft_populated']['home_input'] == '2+simplify(x+x)'
entry('evaluated_draft', '2+simplify(x+x)', 'symbolic', '2+2*x')
assert sessions['equation_draft']['home_input'] == 'x=2'
entry('equation_result', 'x=2', 'symbolic', 'x=2')
assert sessions['home_answer']['history'] == sessions['home_unchanged']['history']
assert main['home_unchanged']['calculator_memory']['Ans'] == main['home_answer']['calculator_memory']['Ans']

reopen = load('_reopen')
assert reopen['home_reopened']['native_session']['history'] == sessions['home_answer']['history']
assert reopen['cas_reopened']['native_session']['history'] == sessions['equation_result']['history']
assert reopen['cas_reopened']['calculator_cas']['last_value'] == {'type': 'symbolic', 'value': 'x=2'}

image = Image.open(root / 'cas_history_simplify_simplified_latest.png').convert('L')
assert image.size == (320, 240)
for left, right in [(55, 103), (108, 157)]:
    assert sum(image.getpixel((x, y)) > 180 for x in range(left, right)
               for y in range(225, 236)) >= 10
print('CAS softkeys simplify latest and selected values, insert editable syntax, and preserve Home and CAS over reopen')
