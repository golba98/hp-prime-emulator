"""Verify the shared Statistics 2Var Autoscale from PPL, the plot menu and reopen."""
import json
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
app = 'Statistics 2Var'
wide = [-20., 20., -20., 20.]
single = [0.940020898642, 3.06666666666, 2.86666666666, 7.13333333333]
combined = [-5.44984326019, 10.5, -12.7333333333, 10.7333333333]


def load(suffix=''):
    report = json.loads((root / f'native_side_statistics_two_scale{suffix}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {item['label']: item['state'] for item in report['snapshots']}


def check_bounds(actual, expected):
    assert len(actual) == len(expected), actual
    for got, want in zip(actual, expected):
        assert abs(float(got) - want) <= max(abs(want), 1.) * 1e-11, (got, want)


def bounds(state, expected, *, ppl=True):
    tools = state['native_tools']
    assert tools['page'] == 'Plot' and tools['error'] is None and not tools['calculating'], tools
    window = tools['memory']['model_windows'][app]
    check_bounds([window[key] for key in ['xmin', 'xmax', 'ymin', 'ymax']], expected)
    if ppl:
        outcome = state['native_session']['last_outcome']
        assert outcome['status'] == 'success', outcome
        result = outcome['typed_result']
        assert result['type'] == 'list' and all(item['type'] == 'real' for item in result['value'])
        check_bounds([item['value'] for item in result['value']], expected)


main = load()
reopened = load('_reopen')['reopened']
bounds(main['wide_plot'], wide)
bounds(main['wide_again'], wide)
bounds(main['single'], single)
bounds(main['two_analyses'], combined)
bounds(main['interactive'], combined, ppl=False)
bounds(main['saved_bounds'], combined)
bounds(reopened, combined)

for state in [main['second_enabled'], reopened]:
    book = state['native_tools']['memory']['models']['statistics2_book']
    assert [analysis['enabled'] for analysis in book['analyses']] == [True, True, False, False, False]
    assert [(analysis['x'], analysis['y']) for analysis in book['analyses'][:2]] == [(0, 1), (3, 4)]
for state in [main['interactive'], main['two_analyses'], reopened]:
    plot = state['native_model_results'][f'{app}:plot']
    assert plot['labels'] == ['S1 observations', 'S1 Linear fit', 'S2 observations', 'S2 Linear fit']
    assert plot['curves'][0] == [[1., 3.], [2., 5.], [3., 7.]]
    assert plot['curves'][2] == [[-5., -12.], [0., 10.], [10., 4.]]
    for line in [plot['curves'][1], plot['curves'][3]]:
        check_bounds([line[0][0], line[-1][0]], combined[:2])

def plot_image(label):
    with Image.open(root / f'stats_two_scale_{label}.png') as image:
        assert image.size == (320, 240)
        return image.crop((0, 0, 320, 219)).convert('RGB').tobytes()


assert plot_image('two_analyses') != plot_image('wide_plot')
assert plot_image('interactive') == plot_image('reopened')
print('Statistics 2Var Autoscale uses both selected series, updates typed PPL bounds, and survives reopen')
