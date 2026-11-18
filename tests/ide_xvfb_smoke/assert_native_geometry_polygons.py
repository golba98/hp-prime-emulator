"""Construct a parallelogram through Cmds and measure it through PPL."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
report = json.loads((root / 'native_side_geometry_polygons.json').read_text())
assert not report['wait_timeouts'], report['wait_timeouts']
states = {item['label']: item['state'] for item in report['snapshots']}
state = states['constructed']
objects = state['native_tools']['memory']['models']['geometry_book']['objects']
assert objects[-1]['source'] == 'parallelogram(GA,GB,GC)', objects
shapes = state['native_model_results']['Geometry:plot']['geometry']['objects']
points = shapes[-1]['value']['Ok']['Polygon']
assert [(p['x'], p['y']) for p in points] == [(-2, 0), (2, 0), (0, 3), (-4, 3)]
closed = states['closed']
objects = closed['native_tools']['memory']['models']['geometry_book']['objects']
assert objects[-1]['source'] == 'polygon(GA,GB,GC,GE)', objects
shapes = closed['native_model_results']['Geometry:plot']['geometry']['objects']
vertices = shapes[-1]['value']['Ok']['Polygon']
assert [(p['x'], p['y']) for p in vertices] == [(-2, 0), (2, 0), (0, 3), (-5, 3)]
circle = states['excircle']['native_model_results']['Geometry:plot']['geometry']['objects'][-1]['value']['Ok']['Circle']
assert math.isclose(circle['center']['x'], math.sqrt(13)) and circle['center']['y'] == 3, circle
assert math.isclose(circle['radius'], 3), circle
outcome = states['values']['native_session']['last_outcome']
assert outcome['status'] == 'success', outcome
assert [float(v['value']) for v in outcome['typed_result']['value']] == [12, 12, 16, 12, 48, 12]
print('Geometry Cmds constructed and measured the expected polygon vertices')
