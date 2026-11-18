"""Real Application Library navigation, touch launch, calculation and restart."""
import json
import sys
from pathlib import Path
from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
created = json.loads((root / 'native_side_gallery.json').read_text())
reopened = json.loads((root / 'native_side_gallery_reopen.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states = {s['label']: s['state'] for s in created['snapshots']}
restored = {s['label']: s['state'] for s in reopened['snapshots']}
for label, state in list(states.items()) + list(restored.items()):
    assert state['native_tools']['error'] is None, (label, state['native_tools']['error'])
    assert not state['native_tools']['calculating'], label
    assert state['native_session']['home_input'] == '12+', label

def memory(label):
    return states[label]['native_tools']['memory']

for label, highlight in [('fixed', 'Function'), ('right', 'Advanced Graphing'),
                         ('down', 'Statistics 2Var'), ('graph3d_highlighted', 'Graph 3D'),
                         ('scrolled', 'Trig Explorer'),
                         ('prefix', 'Spreadsheet')]:
    m = memory(label)
    assert m['selected_app'] == 'Function', label
    assert m['gallery']['highlight'] == highlight, label
    assert states[label]['native_tools']['page'] == 'Apps', label
assert memory('graph3d_highlighted')['gallery']['first_row'] == 2
assert memory('scrolled')['gallery']['first_row'] == 2
assert memory('prefix')['gallery']['first_row'] == 0
assert states['sheet_open']['native_tools']['page'] == 'Numeric'
assert memory('sheet_open')['selected_app'] == 'Spreadsheet'
assert states['computed']['native_model_results']['Spreadsheet']['A1']['Number'] == 5
assert memory('computed')['models']['spreadsheet']['cells']['A1'] == '=SUM({2,3})'
assert memory('alphabetic')['gallery']['sort'] == 'Alphabetic'
assert memory('alphabetic')['gallery']['highlight'] == 'Spreadsheet'
assert memory('polar_highlight')['selected_app'] == 'Spreadsheet'
assert memory('polar_highlight')['gallery']['highlight'] == 'Polar'
assert memory('polar_started')['selected_app'] == 'Polar'
assert states['polar_started']['native_tools']['page'] == 'Symbolic'
assert memory('chronological')['gallery']['sort'] == 'Chronological'
assert memory('chronological')['gallery']['recent'] == ['Polar', 'Spreadsheet']
assert memory('recent')['gallery']['recent'] == ['Function', 'Polar', 'Spreadsheet']
assert states['sort_cancel']['native_tools']['page'] == 'Apps'
assert memory('sort_cancel')['gallery'] == memory('recent')['gallery']
assert memory('fixed_again')['gallery']['sort'] == 'Fixed'
assert memory('final')['gallery']['sort'] == 'Alphabetic'
m = restored['reopened']['native_tools']['memory']
assert m['gallery'] == memory('final')['gallery']
assert m['models']['spreadsheet'] == memory('final')['models']['spreadsheet']
assert restored['reopened_sheet']['native_model_results']['Spreadsheet']['A1']['Number'] == 5
assert restored['reopened_sheet']['native_tools']['memory']['selected_app'] == 'Spreadsheet'

def pixels(label, box):
    state = states[label]
    image = Image.open(root / f'gallery_{label}.png').convert('RGB')
    (x, y), (r, b) = state['lcd_rect']['min'], state['lcd_rect']['max']
    native = image.crop((round(x), round(y), round(r), round(b))).resize((320, 240), Image.Resampling.NEAREST)
    raw = native.crop(box).tobytes()
    return list(zip(raw[::3], raw[1::3], raw[2::3]))

selected = (173, 204, 255)
assert pixels('fixed', (1, 21, 17, 80)).count(selected) > 800
assert pixels('fixed', (80, 21, 96, 80)).count(selected) == 0
assert pixels('scrolled', (1, 154, 17, 212)).count(selected) > 750
assert pixels('sort_menu', (36, 198, 49, 212)).count(selected) > 30
assert sum(b > r + 80 for r, g, b in pixels('fixed', (20, 23, 58, 61))) > 700
assert sum(r > g + 60 for r, g, b in pixels('fixed', (178, 23, 216, 61))) > 700
assert sum(r > g + 60 for r, g, b in pixels('fixed', (257, 23, 295, 61))) > 500
assert sum(max(p) < 100 for p in pixels('fixed', (267, 222, 317, 238))) > 400
print('Application Library: grid, four arrows, prefix, single tap, Start, three sorts, real worksheet result and separate-process restoration passed')
