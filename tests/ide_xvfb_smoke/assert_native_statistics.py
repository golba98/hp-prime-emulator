"""Actual Statistics entry, five weighted analyses, fit menus, plots and reopen."""
import json
import math
import re
import sys
from pathlib import Path
from PIL import Image
root = Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
reference = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/native-statistics-two-reference-controls.json').read_text())
valid_reference = reference['valid_five_s4_blank']
report=json.loads((root/'native_side_statistics.json').read_text())
reopen=json.loads((root/'native_side_statistics_reopen.json').read_text())
assert not report['wait_timeouts'] and not reopen['wait_timeouts']
states={item['label']:item['state'] for item in report['snapshots']}
for label,state in states.items():
    if label == 'five_weighted_error':
        error = state['native_tools']['error']
        assert error and 'frequency' in error.lower() and 'median' in error.lower(), error
    else:
        assert state['native_tools']['error'] is None,(label,state['native_tools']['error'])
    assert not state['native_tools']['calculating'],label

def result(label,name='Statistics 1Var'):
    return states[label]['native_model_results'][name]
def close(a,b):assert math.isclose(a,b,rel_tol=1e-9,abs_tol=1e-9),(a,b)
def column(state,name):
    v=state['calculator_memory'][name]
    assert v['type']=='list'
    return [float(x['value']) for x in v['value']]
def book(label):return states[label]['native_tools']['memory']['models']['statistics2_book']
blank = book('blank_analysis_kept_unchecked')['analyses'][1]
assert not blank['enabled'] and blank['x'] is None and blank['y'] is None
h=result('one_summary')['H1']
assert [h[k] for k in ['count','minimum','quartile1','median','quartile3','maximum','sum','sum_squares','mean']]==[4,1,1.5,2.5,3.5,4,10,30,2.5]
close(h['sample_standard_deviation'],1.29099444874)
h=result('one_weighted')['H1']
assert [h[k] for k in ['count','minimum','median','maximum','mean']]==[4,1,2,3,2]
assert column(states['one_data'],'D1')==[1,2,3,4]
assert column(states['two_data'],'C1')==[1,2,3]
assert column(states['two_data'],'C2')==[3,5,7]
assert column(states['two_make'],'C3')==[1,2,1]
kinds=['Linear','Logarithmic','Exponential','Power','Exponent','Inverse','Logistic','Quadratic','Cubic','Quartic','Trigonometric','MedianMedian','UserDefined']
for index,kind in enumerate(kinds,1):assert book(f'fit_kind_{index}')['analyses'][0]['fit']==kind
user=result('two_userdefined','Statistics 2Var')['S1']
assert user['regression']['user_expression']=='2*X+1'
assert user['summary']['count']==4
close(user['regression']['residual_sum_squares'],0)
# The original rejects explicit frequencies for Median–Median. Keep the invalid
# five-analysis setup as a visible error/recovery phase; clear only S4 through
# the real Symbolic column menu before calculating the valid five fits.
invalid = book('five_weighted_error')
assert invalid == book('analysis_5')
for analysis in invalid['analyses']:
    assert analysis['enabled'] and analysis['frequency'] == 2
expected_recovery = json.loads(json.dumps(invalid))
expected_recovery['analyses'][3]['frequency'] = None
assert book('five_recovery_definitions') == expected_recovery
for label in ['five_weighted_error', 'five_recovery_definitions']:
    for name in ['C1', 'C2', 'C3']:
        assert states[label]['calculator_memory'][name] == states['analysis_5']['calculator_memory'][name]
results=result('five_summaries','Statistics 2Var')
assert set(results)=={'S1','S2','S3','S4','S5'}
for index,kind in enumerate(valid_reference['fits']):
    a=book('five_summaries')['analyses'][index]
    assert (a['enabled'],a['x'],a['y'],a['frequency'],a['fit'])==(True,0,1,None if index == 3 else 2,kind)
    fit=results[f'S{index+1}']
    assert fit['summary']['count']==valid_reference['counts'][index] and fit['regression']['kind']==kind
    assert len(fit['residuals'])==3
close(results['S1']['regression']['coefficients'][0],1)
close(results['S1']['regression']['coefficients'][1],2)
# Independently captured original fit equations validate the computed
# coefficients and every sampled graph point; do not derive the oracle from
# the same native result that supplies the plot.
def original_coefficients(equation, kind):
    number = r'([+-]?[0-9]+(?:\.[0-9]+)?)'
    if kind == 'Power':
        match = re.fullmatch(number + r'\*X\^' + number, equation)
        assert match, equation
        return [float(match[1]), float(match[2])]
    operator = r'/X' if kind == 'Inverse' else r'\*X'
    match = re.fullmatch(number + operator + r'([+-][0-9]+(?:\.[0-9]+)?)', equation)
    assert match, equation
    return [float(match[2]), float(match[1])]
for index, kind in enumerate(valid_reference['fits']):
    expected = original_coefficients(valid_reference['fit_equations'][index], kind)
    actual = results[f'S{index+1}']['regression']['coefficients']
    assert len(actual) == len(expected)
    for left, right in zip(actual, expected):
        close(left, right)
assert states['cell_sin']['native_session']['home_input']=='SIN()'
assert states['cell_sin']['native_session']['cursor']==4
assert states['cell_sin_deleted']['native_session']['home_input']==''
assert column(states['cell_expression'],'C4')==[0]
assert states['cell_expression']['native_session']['ans']==states['two_make']['native_session']['ans']
for name in ['plot_fullscreen','plot_with_fit','plot_zoom','plot_goto']:
    assert states[name]['native_tools']['page']=='Plot'
plot=states['plot_fullscreen']['native_model_results']['Statistics 2Var:plot']
def check_five_fits(plot, bounds):
    assert len(plot['curves']) == len(plot['styles']) == len(plot['labels'])
    for label, style in zip(plot['labels'], plot['styles']):
        assert style in ['Scatter', 'Line'] and (style == 'Scatter') == label.endswith(' observations'), (label, style)
    observations = [(label, curve) for label, style, curve in zip(plot['labels'], plot['styles'], plot['curves']) if style == 'Scatter']
    assert [label for label, _ in observations] == [f'S{i} observations' for i in range(1, 6)]
    assert all(curve == [[1,3],[2,5],[3,7]] for _, curve in observations)
    allowed = {f'S{i} observations' for i in range(1, 6)}
    for index, kind in enumerate(['Linear', 'Power', 'Inverse', 'Median-median', 'Linear'], 1):
        name = f'S{index} {kind} fit'
        allowed.add(name)
        segments = [curve for label, style, curve in zip(plot['labels'], plot['styles'], plot['curves']) if label == name and style == 'Line']
        expected_segments = 2 if kind == 'Inverse' and bounds['xmin'] < 0 < bounds['xmax'] else 1
        assert len(segments) == expected_segments, (name, bounds, segments)
        coefficients = original_coefficients(valid_reference['fit_equations'][index-1], valid_reference['fits'][index-1])
        for curve in segments:
            assert len(curve) >= 2, name
            if kind == 'Inverse':
                assert all(point[0] < 0 for point in curve) or all(point[0] > 0 for point in curve), curve
            for x, y in curve:
                assert math.isfinite(x) and math.isfinite(y), (name, x, y)
                if kind == 'Power':
                    assert x > 0
                    predicted = coefficients[0] * x ** coefficients[1]
                elif kind == 'Inverse':
                    predicted = coefficients[0] + coefficients[1] / x
                else:
                    predicted = coefficients[0] + coefficients[1] * x
                close(y, predicted)
    assert set(plot['labels']) == allowed, plot['labels']
check_five_fits(plot, states['plot_fullscreen']['native_tools']['memory']['model_windows']['Statistics 2Var'])
a=states['plot_fullscreen']['native_tools']['memory']['model_windows']['Statistics 2Var']
b=states['plot_zoom']['native_tools']['memory']['model_windows']['Statistics 2Var']
assert [a[key] for key in ['xmin','xmax','ymin','ymax']] == valid_reference['after_startview_1_1']
close(a['xmax']-a['xmin'],2*(b['xmax']-b['xmin']))
trace=states['plot_goto']['native_plot_trace']
assert trace['enabled']
sample=states['plot_goto']['native_tools']['plot_points'][trace['curve']][trace['sample']]
close(sample[0],2)

def pixels(label,box):
    state=states[label];lcd=state['lcd_rect'];x,y=lcd['min'];r,b=lcd['max']
    bounds=(int(x+box[0]*(r-x)/320),int(y+box[1]*(b-y)/240),int(x+box[2]*(r-x)/320),int(y+box[3]*(b-y)/240))
    raw=Image.open(root/f'statistics_{label}.png').convert('RGB').crop(bounds).tobytes()
    return list(zip(raw[::3],raw[1::3],raw[2::3]))
def colored(p):return sum(max(rgb)-min(rgb)>100 and min(rgb)<160 for rgb in p)
# The Native column menu's first choice clears the source to implicit unit
# weights. This is not a claim that original explicit literal 1 is valid.
assert sum(max(rgb)<80 for rgb in pixels('analysis_5',(230,79,310,94)))>2
assert sum(max(rgb)<80 for rgb in pixels('five_recovery_definitions',(230,79,310,94)))==0
assert colored(pixels('plot_with_fit',(0,0,320,219)))>colored(pixels('plot_fullscreen',(0,0,320,219)))+100
# Fnc must display a calculated equation and OK, rather than the graph panel.
assert sum(max(rgb)<80 for rgb in pixels('plot_function',(0,0,320,35)))>10
assert sum(max(rgb)-min(rgb)>100 for rgb in pixels('plot_function',(0,40,320,210)))<10
restored={i['label']:i['state'] for i in reopen['snapshots']}
state=restored['reopened'];assert state['native_tools']['error'] is None
assert state['native_tools']['memory']['models']['statistics2_book']==book('plot_goto')
for col,expected in [('C1',[1,2,3]),('C2',[3,5,7]),('C3',[1,2,1]),('C4',[0])]:assert column(state,col)==expected
assert state['native_tools']['memory']['statistics_fit_visible'][1]
check_five_fits(restored['reopened_plot']['native_model_results']['Statistics 2Var:plot'], restored['reopened_plot']['native_tools']['memory']['model_windows']['Statistics 2Var'])
print('Statistics: actual grid/Make/expression entry,13 fit choices,weighted Median–Median error/atomic definitions, S4-only recovery, 5 valid fits/pole-separated contours/full-screen plot controls,atomic SIN deletion and separate-process persistence passed')
