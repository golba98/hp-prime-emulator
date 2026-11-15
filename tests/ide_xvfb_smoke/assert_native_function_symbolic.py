"""Production Function rows, inline editing, dependent definitions and restart."""
import json
import sys
from pathlib import Path
from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
created = json.loads((root / 'native_side_function_symbolic.json').read_text())
reopened = json.loads((root / 'native_side_function_symbolic_reopen.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states = {s['label']: s['state'] for s in created['snapshots']}
restored = {s['label']: s['state'] for s in reopened['snapshots']}
drafts = {'editing': '', 'sin': 'SIN()', 'deleted_sin': '', 'invalid': 'X+()',
          'function_menu': '', 'function_inserted': 'F2()'}
for label, state in list(states.items()) + list(restored.items()):
    tools = state['native_tools']
    if label == 'invalid':
        assert tools['error'] and 'expression' in tools['error']
    else:
        assert tools['error'] is None, (label, tools['error'])
    assert not tools['calculating'], label
    assert state['native_session']['home_input'] == drafts.get(label, '12+'), label
    assert state['native_session']['ans']['value'] == '0', label

def memory(label):
    return states[label]['native_tools']['memory']

def curves(label):
    return memory(label)['graphs']['Function']['curves']

assert states['sin']['native_session']['cursor'] == 4
assert memory('f1')['graph_symbolic']['Function']['row'] == 1
assert curves('f1')[0]['expression'] == 'X^2+1'
assert curves('f2')[1]['expression'] == '2*F1(X+1)'
assert memory('f2')['graph_symbolic']['Function']['row'] == 2
assert curves('show') == curves('f2')
assert curves('expanded')[1]['expression'] == '2*((X+1)^2+1)'
assert not curves('green')[0]['enabled'] and curves('green')[1]['enabled']
assert memory('green')['graph_symbolic']['Function']['colors'][1] == 0x008000
assert states['numeric']['native_tools']['table'][:3] == [[0, 4], [1, 10], [2, 20]]
assert states['frozen_after_change']['native_tools']['table'][:3] == [[0, 5, 4], [1, 6, 10], [2, 9, 20]]
assert curves('frozen_after_change')[0]['expression'] == 'X^2+5'
assert curves('frozen_after_change')[1]['expression'] == curves('expanded')[1]['expression']
assert memory('f0')['graph_symbolic']['Function']['row'] == 9
assert memory('f0')['graph_symbolic']['Function']['first'] == 3
assert curves('f0')[9]['expression'] == 'X+10'
assert curves('invalid')[9]['expression'] == curves('cancelled')[9]['expression'] == 'X+10'
assert states['function_inserted']['native_session']['cursor'] == 3
assert states['numeric_function_call']['native_tools']['table'][:3] == [[0, 5, 4, 20, 10], [1, 6, 10, 20, 11], [2, 9, 20, 20, 12]]
assert curves('final')[2]['expression'] == '' and not curves('final')[2]['enabled']
for label in ['reopened', 'reopened_numeric', 'reopened_f0']:
    m = restored[label]['native_tools']['memory']
    assert m['graphs']['Function'] == memory('final')['graphs']['Function'], label
    assert m['graph_symbolic']['Function']['colors'] == memory('final')['graph_symbolic']['Function']['colors']
assert restored['reopened_numeric']['native_tools']['table'][:3] == [[0, 5, 4, 10], [1, 6, 10, 11], [2, 9, 20, 12]]
assert restored['reopened_f0']['native_tools']['memory']['graph_symbolic']['Function']['row'] == 9

def pixels(label, box):
    state = states[label]
    (x, y), (r, b) = state['lcd_rect']['min'], state['lcd_rect']['max']
    image = Image.open(root / f'function_v17_{label}.png').convert('RGB')
    native = image.crop((round(x), round(y), round(r), round(b))).resize((320, 240), Image.Resampling.NEAREST)
    raw = native.crop(box).tobytes()
    return list(zip(raw[::3], raw[1::3], raw[2::3]))

assert pixels('empty', (100, 25, 310, 40)).count((255, 255, 255)) > 2800
assert pixels('empty', (37, 21, 73, 24)).count((173, 204, 255)) > 70
assert sum(g > r + 70 and g > b + 70 for r, g, b in pixels('plot', (0, 50, 319, 219))) > 20
assert sum(g > r + 70 and g > b + 70 for r, g, b in pixels('green', (19, 51, 33, 65))) > 100
assert pixels('show', (0, 0, 320, 219)).count((255, 255, 255)) > 68_000
assert sum(max(p) < 100 for p in pixels('f0', (79, 179, 314, 201))) > 200
print('Function Symbolic: real rows/inline SIN/edit/cancel/Show/Eval, unchecked dependencies, graph colors, F0, function insertion, numeric/plot values and independent restoration passed')
