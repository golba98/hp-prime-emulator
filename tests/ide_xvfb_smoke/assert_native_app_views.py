"""Real PPL app-specific view numbers, actual GUI entry, split plots and reopen."""
import json
import math
import sys
from pathlib import Path
from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
report = json.loads((root / 'native_side_app_views.json').read_text())
reopened = json.loads((root / 'native_side_app_views_reopen.json').read_text())
assert not report['wait_timeouts'] and not reopened['wait_timeouts']
states = {item['label']: item['state'] for item in report['snapshots']}
for app, numbers in [('stats1', range(8,10)), ('stats2', range(8,10)),
                     ('parametric', range(8,14)), ('polar', range(8,12)),
                     ('sequence', range(8,11)), ('advanced', range(8,12)),
                     ('solve', range(8,13))]:
    for number in numbers:
        state = states[f'{app}_{number}']
        assert state['worker_outcome']['kind'] == 'success', (app,number,state['worker_outcome'])
        assert state['worker_outcome']['value'] == '1'
        assert state['native_tools']['error'] is None
        assert not state['native_tools']['calculating']
        expected = 'PlotDetail' if number == 8 else 'PlotTable' if number == 9 and app in ('parametric','polar','sequence') else 'Plot'
        assert state['native_tools']['page'] == expected, (app, number)
for app in ('polar','sequence'):
    state = states[f'{app}_unsupported']
    assert state['worker_outcome']['kind'] == 'success' and state['worker_outcome']['value'] == '0'
    assert state['native_tools']['page'] == 'Symbolic'

assert states['stats1_inputs']['native_tools']['memory']['models']['statistics1_book']['columns'][0] == [1,2,3]
assert states['stats2_inputs']['native_tools']['memory']['models']['statistics2_book']['columns'][:2] == [[1,2,3],[3,5,7]]
for state in (states['stats2_8'], reopened):
    data = state['native_model_results']['Statistics 2Var:plot']
    assert data['curves'][0] == [[1,3],[2,5],[3,7]]
    assert data['styles'] == ['Scatter','Line']
    assert all(abs(y-2*x-1) < 1E-9 for x,y in data['curves'][1])
assert states['stats2_8']['native_tools']['detail_plot'] == states['stats2_zoom']['native_tools']['detail_plot']
a = states['stats2_8']['native_tools']['detail_plot'][0]
b = states['stats2_zoom']['native_tools']['memory']['model_windows']['Statistics 2Var']
assert abs((b['xmax']-b['xmin'])*2-(a['xmax']-a['xmin'])) < 1E-10

parametric = states['parametric_definition']['native_tools']['memory']['graphs']['Parametric']['curves'][0]
assert parametric['expression'] == 'T' and parametric['second'] == 'T^2'
assert states['parametric_9']['native_tools']['table'][:3] == [[0,0,0],[1,1,1],[2,2,4]]
for point in states['parametric_9']['native_tools']['plot_points'][0]:
    assert point is None or abs(point[1]-point[0]**2) < 1E-8
assert states['polar_9']['native_tools']['table'][:3] == [[0,2],[90,2],[180,2]]
for point in states['polar_9']['native_tools']['plot_points'][0]:
    assert point is None or abs(math.hypot(*point)-2) < 1E-8
assert states['sequence_9']['native_tools']['table'][:3] == [[1,1],[2,4],[3,9]]
for point in states['sequence_8']['native_tools']['plot_points'][0]:
    assert point is None or (point[0] == round(point[0]) and abs(point[1]-point[0]**2) < 1E-8)
for app in ('advanced','parametric'):
    decimal = states[f'{app}_{9 if app == "advanced" else 11}']['native_tools']['memory']['graphs']['Advanced Graphing' if app == 'advanced' else 'Parametric']['window']
    assert abs(decimal['xmax']-decimal['xmin']-31.8) < 1E-10
for app, number, name, yspan in [('advanced',10,'Advanced Graphing',6),('parametric',12,'Parametric',21.8),('solve',11,'Solve',11)]:
    tools=states[f'{app}_{number}']['native_tools']['memory']
    window=tools['graphs'][name]['window'] if name in tools['graphs'] else tools['model_windows'][name]
    assert abs(window['xmax']-window['xmin']-318)<1E-10
    assert abs(window['ymax']-window['ymin']-yspan)<1E-10
angle=states['parametric_app_angle']
assert angle['worker_outcome']['kind']=='success' and angle['worker_outcome']['value']=='1'
window=angle['native_tools']['memory']['graphs']['Parametric']['window']
assert abs(window['xmax']-window['xmin']-2650)<1E-9
points = [p for curve in states['advanced_8']['native_tools']['plot_points'] for p in curve if p]
assert points and all(abs(x*x+y*y-4) < 0.01 for x,y in points)
solve = states['solve_8']['native_model_results']['Solve:plot']
assert solve['curves'][0][0] == [1,1] and solve['curves'][0][-1] == [3,9]
assert all(abs(y-4) < 1E-9 for label,curve in zip(solve['labels'],solve['curves']) if label.startswith('Right:') for x,y in curve)

def traced(name):
    state=states[name]
    trace=state['native_plot_trace']
    assert trace['enabled']
    return state['native_tools']['plot_points'][trace['curve']][trace['sample']]
x,y=traced('parametric_table_trace')
assert abs(x-2)<0.02 and abs(y-4)<0.05,(x,y)
x,y=traced('polar_table_trace')
assert abs(x)<0.03 and abs(y-2)<0.03,(x,y)
# Actual scatter dots must be visible in both LCD halves. Mere cached data or
# a successful STARTVIEW reply would not prove a rendered split view.
image=Image.open(root/'native_app_views_stats2_8.png').convert('RGB')
lcd=states['stats2_8']['lcd_rect'];left,top=lcd['min'];right,bottom=lcd['max'];width=right-left;height=bottom-top
for xa,xb in [(0.03,0.47),(0.53,0.97)]:
    region=image.crop((int(left+xa*width),int(top+0.28*height),int(left+xb*width),int(top+0.87*height)))
    pixels=region.tobytes()
    assert sum(b>r+80 and b>g+80 for r,g,b in zip(pixels[::3],pixels[1::3],pixels[2::3]))>5
zoom=Image.open(root/'native_app_views_stats2_zoom.png').convert('RGB')
reference=(int(left+0.03*width),int(top+0.28*height),int(left+0.47*width),int(top+0.87*height))
assert image.crop(reference).tobytes()==zoom.crop(reference).tobytes()
print('Native app views: 26 app-specific actions plus app-angle check, 2 refused numbers, typed GUI data/formulas, Cartesian table trace, visible split plots and reopen passed')
