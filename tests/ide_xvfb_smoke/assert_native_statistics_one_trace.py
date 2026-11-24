"""Actual chart-specific trace navigation against independently captured G1 data."""
import json
import math
import re
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
report = json.loads((root / 'native_side_statistics_one_trace.json').read_text())
reference = json.loads((Path(__file__).parents[1] / 'fixtures/native-statistics-one-trace-controls.json').read_text())
assert not report['wait_timeouts'], report['wait_timeouts']
states = {item['label']: item['state'] for item in report['snapshots']}
for label, state in states.items():
    assert state['native_tools']['error'] is None, (label, state['native_tools']['error'])
    assert not state['native_tools']['calculating'], label


def close(actual, expected):
    assert math.isfinite(actual)
    assert math.isclose(actual, expected, rel_tol=1e-9, abs_tol=1e-9), (actual, expected)


def number(value):
    assert value['kind'] == 'real', value
    return float(value['value'])


def match_sample(actual, expected, case):
    for coordinate in ['x', 'y']:
        if coordinate in expected:
            if expected[coordinate] is None:
                assert actual[coordinate] is None, (case['name'], actual)
            else:
                close(actual[coordinate], expected[coordinate])
    assert math.isfinite(actual['x'])
    assert actual['y'] is None or math.isfinite(actual['y'])
    for field in ['value', 'auxiliary']:
        if field not in expected:
            continue
        if expected[field] is None:
            assert actual.get(field) is None, (case['name'], actual)
        else:
            value = number(actual[field])
            if field == 'auxiliary' and 'auxiliary_display_places' in case:
                assert round(value, case['auxiliary_display_places']) == float(expected[field]), (case['name'], value, expected)
            else:
                close(value, float(expected[field]))
    for field in ['frequency', 'row', 'label']:
        if field in expected:
            assert actual[field] == expected[field], (case['name'], actual, expected)


def normalized(text):
    return re.sub(r'\s+', '', text.replace('−', '-').replace('…', '...'))


def check_footer(state, expected, case):
    caption = state['native_tools']['statistics_trace']
    if case['name'] == 'normal':
        expected_left, expected_right = expected.rsplit(' Y:', 1)
        assert normalized(caption['left']) == normalized(expected_left)
        assert caption['right'].startswith('Y:')
        close(float(caption['right'][2:]), float(expected_right))
    else:
        assert normalized(caption['left'] + ' ' + caption['right']) == normalized(expected), (case['name'], caption, expected)


for case in reference['cases']:
    name = case['name']
    initial = states[f'{name}_0']
    data = initial['native_model_results']['Statistics 1Var:plot']
    assert len(data['statistics_traces']) == 1
    series = data['statistics_traces'][0]
    assert series['analysis'] == 1
    samples = series['samples']
    assert len(samples) == len(case['samples']), (name, samples)
    last = case['samples'][-1]
    for index, expected in enumerate(case['samples']):
        match_sample(samples[index], expected, case)
        state = states[f'{name}_{index}']
        trace = state['native_plot_trace']
        caption = state['native_tools']['statistics_trace']
        assert trace['enabled'] and trace['sample'] == caption['index'] == index
        assert caption['analysis'] == 1
        match_sample(caption['sample'], expected, case)
        selected = state['native_tools']['trace_points'][trace['curve']][trace['sample']]
        close(selected[0], caption['sample']['x'])
        if expected.get('y') is not None and 'y' in expected:
            close(selected[1], expected['y'])
        else:
            assert math.isfinite(selected[1])  # view-only crosshair position; metadata stays undefined
        check_footer(state, case['footers'][index], case)
    state = states[f'{name}_last']
    trace = state['native_plot_trace']
    caption = state['native_tools']['statistics_trace']
    assert trace['enabled'] and trace['sample'] == caption['index'] == len(samples) - 1
    match_sample(caption['sample'], last, case)
    check_footer(state, case['footers'][-1], case)
undefined = states['moving_range_0']['native_tools']['statistics_trace']['sample']
assert undefined['y'] is None and undefined.get('value') is None
assert 'NaN' in states['moving_range_0']['native_tools']['statistics_trace']['left']
off = states['moving_range_off']
on = states['moving_range_on']
assert not off['native_plot_trace']['enabled'] and off['native_plot_trace']['sample'] == 2
assert on['native_plot_trace']['enabled'] and on['native_plot_trace']['sample'] == 2
check_footer(on, next(case for case in reference['cases'] if case['name'] == 'moving_range')['footers'][-1], {'name':'moving_range'})


def footer_pixels(label):
    state = states[label]
    left, top = state['lcd_rect']['min']
    right, bottom = state['lcd_rect']['max']
    image = Image.open(root / f'statistics_one_trace_{label}.png').convert('RGB')
    # The first five softkey positions hold data while the controls are hidden.
    return image.crop((round(left), round(top + (bottom - top) * 220 / 240),
                       round(left + (right - left) * 265 / 320), round(bottom))).tobytes()


assert footer_pixels('moving_range_off') != footer_pixels('moving_range_last')
assert footer_pixels('moving_range_on') == footer_pixels('moving_range_last'), 'Trace on must close controls and restore the selected result footer'
print('Statistics trace: all ten actual chart views plus Moving Range, genuine raw/sorted/bin/quartile/percentage values, undefined first entry, keyboard boundaries and production footer fields passed')
