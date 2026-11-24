"""Real Function setup choices, independent Home settings, defaults and restart."""
import json
import sys
from pathlib import Path
from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
created = json.loads((root / 'native_side_function_setup.json').read_text())
reopened = json.loads((root / 'native_side_function_setup_reopen.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states = {s['label']: s['state'] for s in created['snapshots']}
restored = reopened['snapshots'][-1]['state']
home = states['system']['calculator_home_settings']
assert home == dict(angle='Radians', format=0, digits=4)
for label, state in list(states.items()) + [('reopened', restored)]:
    assert state['native_tools']['error'] is None, (label, state['native_tools']['error'])
    assert state['calculator_home_settings'] == home, label
    assert state['native_session']['home_input'] == ('' if label == 'query' else '12+'), label

def app(label):
    return states[label]['calculator_app_settings'].get('Function', dict(angle=0, format=0, digits=4, complex=0))

assert app('floating') == dict(angle=2, format=5, digits=4, complex=0)
assert app('overrides') == dict(angle=2, format=5, digits=11, complex=1)
for code in range(7):
    assert app(f'format{code}') == dict(angle=2, format=code, digits=11, complex=1)
assert app('digits0') == dict(angle=2, format=6, digits=0, complex=1)
value = states['query']['native_session']['last_outcome']['typed_result']
assert value['type'] == 'list'
assert [v['value'] for v in value['value']] == ['2', '6', '0', '1', '0', '0', '4']
assert app('one_default') == dict(angle=2, format=6, digits=0, complex=0)
assert app('defaults') == dict(angle=0, format=0, digits=4, complex=0)
assert app('final') == dict(angle=2, format=5, digits=11, complex=0)
assert restored['calculator_app_settings']['Function'] == app('final')
assert restored['native_session']['ans'] == states['query']['native_session']['ans']

def pixels(label, box):
    state = states[label]
    (x, y), (r, b) = state['lcd_rect']['min'], state['lcd_rect']['max']
    image = Image.open(root / f'function_setup_{label}.png').convert('RGB')
    native = image.crop((round(x), round(y), round(r), round(b))).resize((320, 240), Image.Resampling.NEAREST)
    raw = native.crop(box).tobytes()
    return list(zip(raw[::3], raw[1::3], raw[2::3]))

assert pixels('system', (40, 120, 300, 190)).count((232, 241, 246)) > 17_000
assert pixels('system', (274, 52, 310, 68)).count((232, 241, 246)) > 500
assert pixels('floating', (274, 52, 296, 68)).count((255, 255, 255)) > 200
assert pixels('format_menu', (155, 50, 165, 66)).count((173, 204, 255)) > 60
assert pixels('digits_menu', (267, 109, 309, 125)).count((173, 204, 255)) > 450
assert sum(max(p) < 100 for p in pixels('system', (55, 222, 103, 238))) > 450
print('Function setup: measured fields, seven formats, digit endpoints, actual PPL values, Home independence, per-field/all defaults and separate-process restoration passed')
