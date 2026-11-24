#!/usr/bin/env python3
"""Assert PPL special-view execution and real GUI zoom against computed data."""
import json
from pathlib import Path
report = json.loads(Path('/tmp/ppl-ide-e2e/native_side_special_views.json').read_text())
assert not report['wait_timeouts'], report['wait_timeouts']
states = {entry['label']: entry['state'] for entry in report['snapshots']}
for view in range(8,14):
    state = states[f'view_{view}']
    assert state['worker_outcome']['value'] == '1', (view,state['worker_outcome'])
    assert state['native_tools']['error'] is None, state['native_tools']
    assert state['native_tools']['page'] == {8:'PlotDetail',9:'PlotTable'}.get(view,'Plot')
left = states['view_8']['native_tools']['detail_plot']
zoom = states['detail_zoom']['native_tools']
assert left == zoom['detail_plot'], 'zoom must preserve reference window and samples'
assert left[1] and zoom['plot_points']
before = left[0]
after = zoom['memory']['graphs']['Function']['window']
assert abs((after['xmax']-after['xmin'])*2-(before['xmax']-before['xmin'])) < 1e-10
for curve in zoom['plot_points']:
    for x,y in curve:
        assert abs(y-x*x)<1e-8
assert states['view_9']['native_tools']['table'][:3] == [[0,0],[1,1],[2,4]]
live = states['live_bounds']
assert live['worker_outcome']['value'] == '{1,4}', live['worker_outcome']
points = live['native_tools']['plot_points'][0]
assert points[0][0] == 1 and points[-1][0] == 4, (points[0],points[-1])
assert states['unsplit']['native_tools']['page'] == 'Plot'
assert states['unsplit']['native_tools']['detail_plot'] is None
print('PPL special views 8–13, GUI detail zoom, immutable reference and plot/table values passed')
