"""Actual Statistics 1Var chart selectors, computed plots and process restart."""
import json
import math
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
created = json.loads((root / 'native_side_statistics_one_charts.json').read_text())
reopened = json.loads((root / 'native_side_statistics_one_charts_reopen.json').read_text())
scale_reference = json.loads((Path(__file__).parents[1] / 'fixtures/native-statistics-one-autoscale-controls.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states = {item['label']: item['state'] for item in created['snapshots']}
restored = {item['label']: item['state'] for item in reopened['snapshots']}
for label, state in list(states.items()) + list(restored.items()):
    assert state['native_tools']['error'] is None, (label, state['native_tools']['error'])
    assert not state['native_tools']['calculating'], label


def analysis(state):
    return state['native_tools']['memory']['models']['statistics1_book']['analyses'][0]


def plot(state):
    return state['native_model_results']['Statistics 1Var:plot']


def summary(state):
    return state['native_model_results']['Statistics 1Var']['H1']


assert [float(value['value']) for value in states['data']['calculator_memory']['D1']['value']] == [1, 2, 4, 7, 9]
assert analysis(states['expression'])['data_source'] == 'D1+1'
assert analysis(states['frequency'])['frequency_source'] == '2'
assert all(analysis(state)['enabled'] for state in states.values())
charts = ['histogram', 'box_whisker', 'normal_probability', 'line', 'bar', 'pareto', 'control', 'dot', 'stem_leaf', 'pie']
rendered = []
for code, chart in enumerate(charts, 1):
    definition = analysis(states[f'chart_{code}_definition'])
    assert definition['chart'] == chart, (code, definition)
    assert definition['option'] == (1 if code in [2, 7, 9] else 0)
    state = states[f'chart_{code}_plot']
    data = plot(state)
    assert data['curves'] and any(data['curves']), (chart, data)
    assert len(data['curves']) == len(data['styles']) == len(data['colors'])
    assert all(math.isfinite(value) for curve in data['curves'] for point in curve for value in point)
    assert all(label.startswith('H1 ') for label in data['labels'])
    window = state['native_tools']['memory']['model_windows']['Statistics 1Var']
    geometry = [point for curve in data['curves'] for point in curve]
    geometry += [point for fill in data.get('fills', []) for point in fill['points']]
    geometry += [annotation['point'] for annotation in data.get('annotations', [])]
    if chart == 'line':
        # The independently executed exact same source demonstrates that the
        # original Line Autoscale uses row-count X bounds and clips its last
        # two data points. Require that behavior rather than imposing a fit.
        original = scale_reference['line_workflow_exact_observations']
        assert data['curves'] == [original['curve']]
        assert all(math.isclose(window[key], expected, rel_tol=1e-9, abs_tol=1e-9)
                   for key, expected in zip(['xmin', 'xmax', 'ymin', 'ymax'], original['bounds']))
        assert [point[0] for point in data['curves'][0] if point[0] > window['xmax']] == [8, 10]
    else:
        assert all(window['xmin'] <= point[0] <= window['xmax']
                   and window['ymin'] <= point[1] <= window['ymax']
                   for point in geometry), (chart, window)
    lcd = state['lcd_rect']
    image = Image.open(root / f'statistics_one_charts_chart_{code}_plot.png').convert('RGB')
    x, y = lcd['min']
    r, b = lcd['max']
    crop = image.crop((round(x), round(y), round(r), round(y + (b - y) * 219 / 240)))
    raw = crop.tobytes()
    pixels = list(zip(raw[::3], raw[1::3], raw[2::3]))
    assert sum(max(pixel) - min(pixel) > 80 for pixel in pixels) > 20, chart
    rendered.append(crop.tobytes())
for code in [2, 7, 9]:
    assert analysis(states[f'chart_{code}_option2'])['option'] == 2
    assert plot(states[f'chart_{code}_option2_plot'])['curves']
assert len(set(rendered)) == 10, 'The ten rendered chart screens must differ'
assert plot(states['chart_9_plot'])['annotations'], 'Stem and leaf needs actual text rows'
assert plot(states['chart_10_plot'])['fills'], 'Pie needs visible sectors'
assert analysis(states['color'])['color'] == 0xff0000
result = summary(states['summary'])
assert [result[key] for key in ['count', 'minimum', 'maximum', 'sum', 'mean']] == [10, 2, 10, 56, 5.6]
assert result['_display']['mean'] == '5.6'
assert analysis(restored['reopened']) == analysis(states['saved'])
assert restored['reopened_plot']['native_tools']['memory']['model_windows']['Statistics 1Var'] == states['chart_10_plot']['native_tools']['memory']['model_windows']['Statistics 1Var']
assert restored['reopened']['calculator_memory']['D1'] == states['saved']['calculator_memory']['D1']
assert restored['reopened']['native_tools']['memory']['models']['statistics1_book']['result_values'] == states['saved']['native_tools']['memory']['models']['statistics1_book']['result_values']
assert summary(restored['reopened_summary']) == result
assert plot(restored['reopened_plot'])['fills']
# The original Pie uses its category palette and black outlines, independently
# of H1's selected color; the Dot chart is the actual color-rendering check.
reference = json.loads((Path(__file__).parents[1] / 'fixtures/native-statistics-one-workflow-controls.json').read_text())
pale_red = reference['chart_observations']['pale_fills'][1]
assert plot(states['colored_dot'])['styles'] == ['TraceOnly']
assert plot(states['colored_dot'])['colors'][0] == 0xff0000
assert {column['color'] for column in plot(states['colored_dot'])['dots']} == {pale_red}
assert sum(column['count'] for column in plot(states['colored_dot'])['dots']) == 10
raw = Image.open(root / 'statistics_one_charts_colored_dot.png').convert('RGB').tobytes()
assert sum(red > green + 40 and red > blue + 40
           for red, green, blue in zip(raw[::3], raw[1::3], raw[2::3])) > 100
assert set(plot(restored['reopened_plot'])['colors']) == {0}
assert [fill['color'] for fill in plot(restored['reopened_plot'])['fills'][:3]] == reference['chart_observations']['pale_fills']
print('Statistics 1Var: ten actual chart selectors/plots, six option choices, expression/frequency edits, color, calculated summary and separate-process restore passed')
