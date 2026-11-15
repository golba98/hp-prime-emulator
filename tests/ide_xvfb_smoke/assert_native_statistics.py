"""Actual Statistics entry, five weighted analyses, fit menus, plots and reopen."""
import json
import math
import sys
from pathlib import Path
from PIL import Image
root = Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
report=json.loads((root/'native_side_statistics.json').read_text())
reopen=json.loads((root/'native_side_statistics_reopen.json').read_text())
assert not report['wait_timeouts'] and not reopen['wait_timeouts']
states={item['label']:item['state'] for item in report['snapshots']}
for label,state in states.items():
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
results=result('five_summaries','Statistics 2Var')
assert set(results)=={'S1','S2','S3','S4','S5'}
for index,kind in enumerate(['Linear','Power','Inverse','MedianMedian','Linear']):
    a=book('five_summaries')['analyses'][index]
    assert (a['enabled'],a['x'],a['y'],a['frequency'],a['fit'])==(True,0,1,2,kind)
    fit=results[f'S{index+1}']
    assert fit['summary']['count']==4 and fit['regression']['kind']==kind
    assert len(fit['residuals'])==3
close(results['S1']['regression']['coefficients'][0],1)
close(results['S1']['regression']['coefficients'][1],2)
assert states['cell_sin']['native_session']['home_input']=='SIN()'
assert states['cell_sin']['native_session']['cursor']==4
assert states['cell_sin_deleted']['native_session']['home_input']==''
assert column(states['cell_expression'],'C4')==[0]
assert states['cell_expression']['native_session']['ans']==states['two_make']['native_session']['ans']
for name in ['plot_fullscreen','plot_with_fit','plot_zoom','plot_goto']:
    assert states[name]['native_tools']['page']=='Plot'
plot=states['plot_fullscreen']['native_model_results']['Statistics 2Var:plot']
assert len(plot['curves'])==10 and plot['styles']==['Scatter','Line']*5
for label,curve in zip(plot['labels'],plot['curves']):
    if label.endswith('observations'):assert curve==[[1,3],[2,5],[3,7]]
a=states['plot_fullscreen']['native_tools']['memory']['model_windows']['Statistics 2Var']
b=states['plot_zoom']['native_tools']['memory']['model_windows']['Statistics 2Var']
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
assert colored(pixels('plot_with_fit',(0,0,320,219)))>colored(pixels('plot_fullscreen',(0,0,320,219)))+100
# Fnc must display a calculated equation and OK, rather than the graph panel.
assert sum(max(rgb)<80 for rgb in pixels('plot_function',(0,0,320,35)))>10
assert sum(max(rgb)-min(rgb)>100 for rgb in pixels('plot_function',(0,40,320,210)))<10
restored={i['label']:i['state'] for i in reopen['snapshots']}
state=restored['reopened'];assert state['native_tools']['error'] is None
assert state['native_tools']['memory']['models']['statistics2_book']==book('plot_goto')
for col,expected in [('C1',[1,2,3]),('C2',[3,5,7]),('C3',[1,2,1]),('C4',[0])]:assert column(state,col)==expected
assert state['native_tools']['memory']['statistics_fit_visible'][1]
assert len(restored['reopened_plot']['native_model_results']['Statistics 2Var:plot']['curves'])==10
print('Statistics: actual grid/Make/expression entry,13 fit choices,5 weighted analyses,computed results/full-screen plot controls,atomic SIN deletion and separate-process persistence passed')
