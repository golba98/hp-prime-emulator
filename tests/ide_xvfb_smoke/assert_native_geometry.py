"""Real Geometry forms, pointer constructions, dependency updates and restart."""
import json
import math
import sys
from pathlib import Path
from PIL import Image
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
def load(name):
    report=json.loads((root/f'native_side_{name}.json').read_text());assert not report['wait_timeouts'],report['wait_timeouts']
    return {s['label']:s['state'] for s in report['snapshots']}
s=load('geometry');r=load('geometry_reopen')
def book(state):return state['native_tools']['memory']['models']['geometry_book']
def scene(state):
    values=state['native_model_results']
    return values.get('Geometry') or values['Geometry:plot']['geometry']
def obj(state,name):return next(o for o in scene(state)['objects'] if o['name']==name)['value']['Ok']
def close(a,b):assert math.isclose(a,b,rel_tol=1e-9,abs_tol=1e-9),(a,b)
for label,state in s.items():
    error=state['native_tools']['error']
    if label=='invalid_self_reference':assert error and 'Circular' in error,error
    else:assert error is None,(label,error)
assert book(s['empty_plot'])['objects']==[]
assert s['empty_plot']['native_tools']['page']=='Plot'
assert obj(s['definitions'],'A')['Point']=={'x':3,'y':4}
assert obj(s['definitions'],'C')['Point']=={'x':5,'y':5}
close(scene(s['distance'])['measurements'][0]['Ok']['Number']['re'],5)
assert not book(s['distance'])['measurements'][0]['visible']
close(scene(s['area'])['measurements'][1]['Ok']['Number']['re'],20*math.pi)
assert obj(s['dragged_point'],'A')['Point']=={'x':5,'y':4}
assert obj(s['dragged_point'],'C')['Point']=={'x':6,'y':5}
close(scene(s['dragged_point'])['measurements'][0]['Ok']['Number']['re'],math.sqrt(41))
close(scene(s['dragged_point'])['measurements'][1]['Ok']['Number']['re'],8*math.pi)
assert next(o for o in book(s['point_color'])['objects'] if o['name']=='A')['color']==255
assert obj(s['point_tool'],'F')['Point']=={'x':-3,'y':-2}
assert next(o for o in book(s['segment_tool'])['objects'] if o['name']=='G')['source']=='segment(GF,GA)'
assert obj(s['segment_tool'],'G')['Line']['a']=={'x':-3,'y':-2}
assert obj(s['segment_tool'],'G')['Line']['b']=={'x':5,'y':4}
names=[o['name'] for o in book(s['renamed'])['objects']];assert names==['K','B','C','D','E','F','G'],names
assert book(s['renamed'])['measurements'][0]['source']=='distance(GK,0)'
assert next(o for o in book(s['renamed'])['objects'] if o['name']=='C')['source']=='midpoint(GK,GB)'
assert [o['name'] for o in book(s['inserted'])['objects']]==['A','K','B','C','D','E','F','G']
assert obj(s['inserted'],'A')['Point']=={'x':1,'y':1}
assert book(s['labelled'])['measurements'][0]['label']=='Origin distance'
assert book(s['labelled'])['measurements'][0]['visible']
assert next(o for o in book(s['invalid_self_reference'])['objects'] if o['name']=='K')['source']=='point(5,4)'
assert obj(s['recovered'],'K')['Point']=={'x':5,'y':4}
assert all('Ok' in o['value'] for o in scene(s['final_plot'])['objects'])
assert s['final_plot']['native_session']['ans']==s['empty_plot']['native_session']['ans']
assert not any(name in s['final_plot']['calculator_memory'] for name in ['GA','GB','GK'])
for label,state in r.items():
    assert state['native_tools']['error'] is None,(label,state['native_tools']['error'])
    assert book(state)==book(s['final_plot'])
    assert obj(state,'K')['Point']=={'x':5,'y':4}
    close(scene(state)['measurements'][0]['Ok']['Number']['re'],math.sqrt(41))
# There must be a visible construction in the actual captured canvas.
def lcd_pixels(label):
    state=s[label];left,top=state['lcd_rect']['min'];right,bottom=state['lcd_rect']['max'];image=Image.open(root/f'geometry_{label}.png').convert('RGB')
    raw=image.crop((int(left),int(top),int(right),int(top+(bottom-top)*219/240))).tobytes()
    return zip(raw[::3],raw[1::3],raw[2::3])
assert sum(red>150 and green<120 and blue<120 for red,green,blue in lcd_pixels('final_plot'))>10
assert sum(blue>150 and red<120 and green<120 for red,green,blue in lcd_pixels('final_plot'))>3
print('Geometry: actual definitions/measurements,point drag/dependencies,Cmds point/segment,colors,rename/insert,label,atomic invalid edit recovery and separate-process persistence passed')
