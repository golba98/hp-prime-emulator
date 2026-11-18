"""Statistics 1Var fractional weights through original numeric controls and Native UI."""
import json
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
fixture = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-statistics-one-fractional-controls.json').read_text())
expected = next(case['results'] for case in fixture['summary_cases'] if case['case'] == 'unequal_fractions')


def states(scenario):
    report = json.loads((root / f'native_side_{scenario}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {entry['label']: entry['state'] for entry in report['snapshots']}


creation = states('statistics_fractional')
reopen = states('statistics_fractional_reopen')
for state in (creation['ppl'], reopen['reopened_ppl']):
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    result = [float(value['value']) for value in outcome['typed_result']['value']]
    assert result == [float(expected[variable]) for variable in ['NbItem', 'Q₁', 'MedVal', 'Q₃', 'MeanX']]

for state in (creation['numeric'], reopen['reopened_numeric']):
    assert state['native_tools']['page'] == 'Numeric'
    assert state['native_tools']['error'] is None, state['native_tools']['error']
    summary = state['native_model_results']['Statistics 1Var']['H1']
    assert float(summary['count']) == 5.5
    assert summary['_display']['mean'] == expected['MeanX']
    assert [float(value['value']) for value in state['calculator_memory']['D2']['value']] == [0.5, 1.25, 2.25, 1.5]

for state, label in ((creation['dot'], 'dot'), (reopen['reopened_dot'], 'reopened_dot')):
    assert state['native_tools']['page'] == 'Plot'
    assert state['native_tools']['error'] is None, state['native_tools']['error']
    data = state['native_model_results']['Statistics 1Var:plot']
    assert [(c['count'], c['fraction']) for c in data['dots']] == [(0, 0.5), (1, 0.25), (2, 0.25), (1, 0.5)]

    # Examine the first half disk away from its clipping edge, trace cursor
    # and grid lines. The inside of the lower half receives the fill color.
    window = state['native_tools']['memory']['model_windows']['Statistics 1Var']
    left, top = state['lcd_rect']['min']
    right, bottom = state['lcd_rect']['max']
    image = Image.open(root / f'statistics_fractional_{label}.png').convert('RGB')
    color = data['dots'][0]['color']
    expected_rgb = (color >> 16, (color >> 8) & 255, color & 255)

    def projected(x, y):
        return (
            round(left + (x - window['xmin']) / (window['xmax'] - window['xmin']) * (right - left)),
            round(top + (window['ymax'] - y) / (window['ymax'] - window['ymin']) * (bottom - top) * 220 / 240),
        )

    assert image.getpixel(projected(1.12, 0.24)) == expected_rgb
    assert image.getpixel(projected(1.12, 0.74)) != expected_rgb

trace = creation['trace']['native_tools']['statistics_trace']
assert trace['analysis'] == 1 and trace['right'] == 'F:0.5', trace
assert float(trace['sample']['fractional_frequency']['value']) == 0.5
print('Fractional Statistics 1Var PPL, Numeric Stats, partial Dot, Trace and restart passed')
