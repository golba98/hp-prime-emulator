"""Real Statistics Autoscale, live PPL bounds, grouped large-Dot tracing and restart."""
import json
import math
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
created = json.loads((root / 'native_side_statistics_one_scale.json').read_text())
reopened = json.loads((root / 'native_side_statistics_one_scale_reopen.json').read_text())
reference = json.loads((Path(__file__).parents[1] / 'fixtures/native-statistics-one-autoscale-controls.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states = {item['label']: item['state'] for item in created['snapshots']}
restored = {item['label']: item['state'] for item in reopened['snapshots']}
for label, state in list(states.items()) + list(restored.items()):
    assert state['native_tools']['error'] is None, (label, state['native_tools']['error'])
    assert not state['native_tools']['calculating'], label


def analyses(state):
    return state['native_tools']['memory']['models']['statistics1_book']['analyses']


def plot(state):
    return state['native_model_results']['Statistics 1Var:plot']


def window(state):
    saved = state['native_tools']['memory']['model_windows']['Statistics 1Var']
    return [saved[key] for key in ['xmin', 'xmax', 'ymin', 'ymax']]


def close(actual, expected):
    assert len(actual) == len(expected), (actual, expected)
    assert all(math.isclose(a, b, rel_tol=1e-9, abs_tol=1e-9) for a, b in zip(actual, expected)), (actual, expected)


def raw_numbers(source):
    return json.loads(source.replace('−', '-').replace('{', '[').replace('}', ']'))


fractional = states['fractional_histogram']
fractional_bounds = raw_numbers(fractional['worker_outcome']['value'])
fractional_control = reference['fractional_histogram_observations']
close(fractional_bounds, fractional_control['bounds'] + fractional_control['histogram'])
close(window(fractional), fractional_control['bounds'])
assert sorted(max(point[1] for point in fill['points']) for fill in plot(fractional)['fills']) == fractional_control['bins']

multi_case = next(case for case in reference['cases'] if case['name'] == 'autoscale_two_analyses_all_types')
bar_control = next(row for row in multi_case['windows'] if row[0] == 5)
first, second = analyses(states['multi_definitions'])[:2]
assert first['enabled'] and second['enabled']
assert first['chart'] == second['chart'] == 'bar'
assert first['frequency_source'] == second['frequency_source'] == '2'
assert second['data_source'] == 'D2'
close(window(states['multi_scaled']), bar_control[1:5])
assert {label.split()[0] for label in plot(states['multi_scaled'])['labels']} == {'H1', 'H2'}
assert max(point[1] for curve in plot(states['multi_scaled'])['curves'] for point in curve) == 400
bounds = raw_numbers(states['multi_ppl_bounds']['worker_outcome']['value'])
# The original multi-type fixture visits Histogram before Bar; its final
# three values retain that earlier Histogram state. This workflow deliberately
# visited the fractional Histogram instead. Bar must preserve those settings.
preserved_histogram = fractional_control['histogram'][:2] + [1]
close(bounds, bar_control[1:5] + preserved_histogram)
close(window(states['multi_ppl_bounds']), bar_control[1:5])
histogram = states['multi_ppl_bounds']['native_tools']['memory']['models']['histogram']
close([histogram[key] for key in ['minimum', 'maximum', 'width']], preserved_histogram)
unit = analyses(states['unit_frequency'])[0]
assert unit['enabled'] and unit.get('frequency_source', '') == '' and unit['frequency'] is None
assert states['unit_frequency']['worker_outcome']['kind'] == 'success', states['unit_frequency']['worker_outcome']
large = analyses(states['large_dot_definition'])
assert large[0]['enabled'] and not large[1]['enabled']
assert large[0]['chart'] == 'dot' and large[0]['frequency_source'] == '10001'
dot_control = next(control for control in reference['controls'] if control['name'] == 'dot_trace_autoscale_source')
close(window(states['large_dot_scaled']), raw_numbers(dot_control['raw_result']))
expected_trace = reference['dot_trace_observations']['right_steps']
for label, expected in zip(['dot_trace_0', 'dot_trace_1', 'dot_trace_2', 'dot_trace_3'], expected_trace):
    state = states[label]
    data = plot(state)
    assert data['styles'] == ['TraceOnly'] and data['curves'] == [expected_trace[:2]]
    assert [(column['x'], column['count']) for column in data['dots']] == [(1, 10001), (3, 20002)]
    assert sum(column['count'] for column in data['dots']) == 30003
    trace = state['native_plot_trace']
    assert trace['enabled']
    assert state['native_tools']['trace_points'][trace['curve']][trace['sample']] == expected
for label, expected in [('dot_goto_1', [1, 10001]), ('dot_goto_3', [3, 20002])]:
    state = states[label]
    trace = state['native_plot_trace']
    assert trace['enabled']
    assert state['native_tools']['trace_points'][trace['curve']][trace['sample']] == expected
close(window(states['dot_decimal']), [-15.9, 15.9, -10.9, 10.9])
assert [(column['x'], column['count']) for column in plot(states['dot_decimal'])['dots']] == [(1, 10001), (3, 20002)]
# The real Decimal preset exposes finite disks from the huge compact columns.
# Require visible colored pixels inside the plotting surface, independently of
# the metadata/trace checks above.
image = Image.open(root / 'statistics_one_scale_dot_decimal.png').convert('RGB')
left, top = states['dot_decimal']['lcd_rect']['min']
right, bottom = states['dot_decimal']['lcd_rect']['max']
raw = image.crop((round(left), round(top), round(right), round(top + (bottom - top) * 219 / 240))).tobytes()
assert sum(max(pixel) - min(pixel) > 40 for pixel in zip(raw[::3], raw[1::3], raw[2::3])) > 100
assert analyses(restored['reopened']) == analyses(states['dot_decimal'])
for name in ['D1', 'D2']:
    assert restored['reopened']['calculator_memory'][name] == states['dot_decimal']['calculator_memory'][name]
close(window(restored['reopened_decimal']), window(states['dot_decimal']))
assert plot(restored['reopened_decimal'])['dots'] == plot(states['dot_decimal'])['dots']
close(window(restored['reopened_autoscale']), raw_numbers(dot_control['raw_result']))
print('Statistics exact Autoscale: both checked Bar analyses, immediate PPL bounds/histogram, unit-frequency normalization, 30003-observation grouped Dot tracing/GoTo/visible zoom and separate-process restore passed')
