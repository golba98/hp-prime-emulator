"""Actual Statistics Defn controls, traced owner, Show outliers and process restore."""
import json
import math
import re
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
created = json.loads((root / 'native_side_statistics_one_definition.json').read_text())
reopened = json.loads((root / 'native_side_statistics_one_definition_reopen.json').read_text())
reference = json.loads((Path(__file__).parents[1] / 'fixtures/native-statistics-one-trace-controls.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states = {item['label']: item['state'] for item in created['snapshots']}
restored = {item['label']: item['state'] for item in reopened['snapshots']}
for label, state in list(states.items()) + list(restored.items()):
    assert state['native_tools']['error'] is None, (label, state['native_tools']['error'])
    assert not state['native_tools']['calculating'], label


def normalized(text):
    return re.sub(r'\s+', '', text.replace('−', '-'))


def definition(state):
    return state['native_tools']['statistics_plot_definition']


def trace(state):
    return state['native_tools']['statistics_trace']


def value(sample):
    encoded = sample.get('value')
    if encoded is None:
        return None
    assert encoded['kind'] == 'real'
    return float(encoded['value'])


def book(state):
    return state['native_tools']['memory']['models']['statistics1_book']


def footer_pixels(label):
    state = states[label]
    left, top = state['lcd_rect']['min']
    right, bottom = state['lcd_rect']['max']
    image = Image.open(root / f'statistics_one_definition_{label}.png').convert('RGB')
    return image.crop((round(left), round(top + (bottom - top) * 220 / 240),
                       round(left + (right - left) * 265 / 320), round(bottom))).tobytes()


controls = reference['definition_controls']
for code, expected in controls['captions'].items():
    state = states[f'caption_{code}']
    assert normalized(definition(state)) == normalized(expected), (code, definition(state), expected)
    assert book(state)['analyses'][0]['data_source'] == controls['data_source']
    assert book(state)['analyses'][0]['frequency_source'] == controls['frequency']
    assert [float(item['value']) for item in state['calculator_memory']['D1']['value']] == [float(item) for item in controls['data']]
line_caption = controls['captions']['4']
for label in ['line_defn_initial', 'line_defn_right', 'line_defn_again', 'line_defn_h1', 'line_before_home']:
    assert normalized(definition(states[label])) == normalized(line_caption), (label, definition(states[label]))
assert trace(states['line_defn_right'])['index'] == trace(states['line_defn_again'])['index'] == 2
assert value(trace(states['line_defn_right'])['sample']) == 3
assert footer_pixels('line_defn_right') == footer_pixels('line_defn_again'), 'Reselecting Defn must preserve the caption'
assert normalized(definition(states['line_defn_h2'])) == normalized('H2: D1+2,2')
assert trace(states['line_defn_h2'])['analysis'] == 2 and value(trace(states['line_defn_h2'])['sample']) == 4
assert trace(states['line_defn_h1'])['analysis'] == 1
assert not states['line_defn_trace_off']['native_plot_trace']['enabled']
assert states['line_defn_trace_off']['native_plot_trace']['sample'] == 2
assert definition(states['line_defn_trace_off']) is None
assert definition(states['line_defn_trace_on']) is None
assert states['line_defn_trace_on']['native_plot_trace']['enabled']
assert trace(states['line_defn_trace_on'])['index'] == 2
assert normalized(trace(states['line_defn_trace_on'])['left'] + trace(states['line_defn_trace_on'])['right']) == 'H1[3]:3F:2'
assert footer_pixels('line_defn_trace_on') != footer_pixels('line_defn_again')
# X=6 belongs to H2, while Go To from H1 must stay on its closest sample, X=5.
line_goto = trace(states['line_goto_h1'])
assert line_goto['analysis'] == 1 and line_goto['index'] == 0
assert value(line_goto['sample']) == 5 and line_goto['sample']['frequency'] == 2
for label in ['line_after_home', 'control_after_home']:
    assert definition(states[label]) is None, label
    assert states[label]['native_plot_trace']['sample'] == states[label]['native_plot_trace']['curve'] == 0
for label in ['control_defn_0', 'control_defn_right', 'control_defn_again']:
    assert normalized(definition(states[label])) == normalized('H1: D1')
assert trace(states['control_defn_0'])['sample']['y'] is None
assert trace(states['control_defn_0'])['sample'].get('value') is None
assert trace(states['control_defn_right'])['index'] == 1 and value(trace(states['control_defn_right'])['sample']) == 3
assert footer_pixels('control_defn_right') == footer_pixels('control_defn_again')

outlier = reference['box_outlier_control']
analysis = book(states['outlier_definitions'])['analyses'][0]
assert analysis['enabled'] and analysis['chart'] == 'box_whisker' and analysis['option'] == 2
assert analysis.get('frequency_source', '') == '' and analysis['frequency'] is None
assert [float(item['value']) for item in states['outlier_definitions']['calculator_memory']['D1']['value']] == [float(item) for item in outlier['data']]
for index, expected in enumerate(outlier['samples']):
    state = states[f'outlier_{index}']
    selected = trace(state)
    assert selected['analysis'] == 1 and selected['index'] == index
    assert selected['sample']['label'] == expected['label']
    assert math.isclose(value(selected['sample']), float(expected['value']), rel_tol=1e-12)
    assert normalized(selected['left'] + selected['right']) == normalized(expected['footer'])
assert trace(states['outlier_clamped'])['index'] == 5
assert value(trace(states['outlier_clamped'])['sample']) == 100
plot = states['outlier_0']['native_model_results']['Statistics 1Var:plot']
assert len(plot['statistics_traces'][0]['samples']) == 6
assert any(label.endswith('Outliers') and any(point[0] == 100 for point in curve)
           for label, curve in zip(plot['labels'], plot['curves']))
assert any(label.endswith('Maximum') and all(point[0] == 7 for point in curve)
           for label, curve in zip(plot['labels'], plot['curves']))
# Inspect the actual nonselected outlier glyph, not just its trace metadata.
state = states['outlier_0']
point = plot['statistics_traces'][0]['samples'][-1]
window = state['native_tools']['memory']['model_windows']['Statistics 1Var']
assert window['xmin'] <= point['x'] <= window['xmax'] and window['ymin'] <= point['y'] <= window['ymax']
left, top = state['lcd_rect']['min']
right, bottom = state['lcd_rect']['max']
x = left + (point['x'] - window['xmin']) / (window['xmax'] - window['xmin']) * (right - left)
y = top + (window['ymax'] - point['y']) / (window['ymax'] - window['ymin']) * (bottom - top) * 220 / 240
image = Image.open(root / 'statistics_one_definition_outlier_0.png').convert('RGB')
raw = image.crop((round(x) - 5, round(y) - 5, round(x) + 6, round(y) + 6)).tobytes()
assert sum(min(pixel) < 160 for pixel in zip(raw[::3], raw[1::3], raw[2::3])) > 2
assert normalized(definition(states['outlier_definition'])) == 'H1:D1'
assert book(restored['reopened'])['analyses'] == book(states['outlier_definition'])['analyses']
assert restored['reopened']['calculator_memory']['D1'] == states['outlier_definition']['calculator_memory']['D1']
assert definition(restored['reopened_box']) is None
assert restored['reopened_box']['native_plot_trace']['sample'] == 0
assert trace(restored['reopened_outlier'])['index'] == 5 and value(trace(restored['reopened_outlier'])['sample']) == 100
assert normalized(definition(restored['reopened_definition'])) == 'H1:D1'
assert definition(restored['reopened_after_home']) is None and restored['reopened_after_home']['native_plot_trace']['sample'] == 0
print('Statistics Defn: ten original captions, current H1/H2 source, Right/idempotency, Trace/Home reset, real Show outliers Max7/Outlier100 rendering/trace and separate-process persistence passed')
