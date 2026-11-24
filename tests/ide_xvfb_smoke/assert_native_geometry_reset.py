"""Verify explicit Geometry reset, cancellation, scope and persistence."""
import json
from pathlib import Path
import sys

from PIL import Image, ImageChops


ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
CUSTOM = [-5., -6., .2, 2., 3., 1., 1., 1., 1., 1.]
DEFAULTS = [-16., -11., .1, 1., 1., 0., 0., 0., 0., 0.]
OTHER_DATA = [246813579., -12.3, 49., 9876.]


def snapshots(suffix=''):
    report = json.loads((ROOT / f'native_side_geometry_reset{suffix}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {s['label']: s['state'] for s in report['snapshots']}


def tools(state):
    return state['native_tools']


def book(state):
    return tools(state)['memory']['models']['geometry_book']


def typed(state):
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    result = outcome['typed_result']
    assert result['type'] == 'list', result
    return result['value']


def numeric(entries):
    assert all(v['type'] == 'real' for v in entries), entries
    return [float(v['value']) for v in entries]


def exact_text(entry):
    assert entry['type'] == 'string', entry
    return entry['value']


def crop_picture(label, state):
    (left, top), (right, bottom) = state['lcd_rect']['min'], state['lcd_rect']['max']
    bounds = tuple(round(origin + extent * fraction) for origin, extent, fraction in (
        (left, right-left, 26/320), (top, bottom-top, 72/240),
        (left, right-left, 295/320), (top, bottom-top, 171/240),
    ))
    with Image.open(ROOT / f'geometry_reset_{label}.png') as image:
        return image.convert('RGB').crop(bounds)


states = snapshots()
reopened = snapshots('_reopen')
for label, state in [*states.items(), *reopened.items()]:
    assert tools(state)['error'] is None and not tools(state)['calculating'], label
    assert tools(state)['memory']['selected_app'] == 'Geometry', label
    if label in ('library', 'cancel_prompt', 'canceled', 'confirm_prompt', 'confirmed'):
        assert tools(state)['page'] == 'Apps', label
        assert tools(state)['memory']['gallery']['highlight'] == 'Geometry', label

authored = book(states['measured'])
assert [v['source'] for v in authored['objects']] == [
    'point(2,3)', 'point(4,5)', 'segment(GA,GB)',
]
assert [v['source'] for v in authored['measurements']] == ['distance(GA,GB)']
for label in ('before', 'library', 'cancel_prompt', 'canceled', 'after_cancel', 'confirm_prompt'):
    assert book(states[label]) == authored, label
before = typed(states['before'])
assert typed(states['after_cancel']) == before
assert numeric(before[:10]) == CUSTOM
assert all(source['source'] in exact_text(before[10]) for source in authored['objects'])
assert numeric(before[11:]) == [0., 0., 314159., *OTHER_DATA]

library = crop_picture('library', states['library'])
for label in ('cancel_prompt', 'confirm_prompt'):
    assert ImageChops.difference(library, crop_picture(label, states[label])).getbbox(), label
for label in ('canceled', 'confirmed'):
    assert not ImageChops.difference(library, crop_picture(label, states[label])).getbbox(), label

after = typed(states['after'])
assert typed(reopened['reopened_values']) == after
assert numeric(after[:10]) == DEFAULTS
assert exact_text(after[10]) == ''
assert numeric(after[11:13]) == [1., 2.]
assert exact_text(after[13]) == 'missing'
assert numeric(after[14:]) == OTHER_DATA
for label in ('confirmed', 'after', 'symbolic_empty', 'numeric_empty', 'plot_empty'):
    assert book(states[label]) == {'objects': [], 'measurements': []}, label
for label, state in reopened.items():
    assert book(state) == {'objects': [], 'measurements': []}, label

for label, page in (('symbolic', 'Symbolic'), ('numeric', 'Numeric'), ('plot', 'Plot')):
    assert tools(states[label+'_empty'])['page'] == page
    assert tools(reopened['reopened_'+label])['page'] == page
for state in [states['plot_empty'], reopened['reopened_plot']]:
    window = tools(state)['memory']['model_windows']['Geometry']
    assert [window[key] for key in ('xmin','xmax','ymin','ymax')] == [-16., 15.9, -11., 10.9]
    scene = state['native_model_results']['Geometry:plot']['geometry']
    assert scene == {'objects': [], 'measurements': []}
print('Geometry Reset passed for explicit confirmation, cancellation, scoped data, empty views and restart')
