"""Function's real Numeric grid/settings, worker validation and saved state."""
import json
import math
import sys
from pathlib import Path
from PIL import Image
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
created=json.loads((root/'native_side_function_numeric.json').read_text())
reopened=json.loads((root/'native_side_function_numeric_reopen.json').read_text())
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
states={s['label']:s['state'] for s in created['snapshots']}
restored={s['label']:s['state'] for s in reopened['snapshots']}
def memory(label):return states[label]['native_tools']['memory']
def numeric(label):return memory(label)['function_numeric']
def graph(label):return memory(label)['graphs']['Function']
def settings(label):return states[label]['calculator_app_numeric_settings']['Function']
def table(label):return states[label]['native_tools']['table']
for label,state in states.items():
    assert not state['worker_alive'] and not state['native_tools']['calculating'],label
    assert not any(k in state['calculator_memory'] for k in ['NumStart','NumStep','NumZoom','NumType','NumIndep']),label
    if label=='invalid_step':
        assert 'positive' in state['native_tools']['error']
        assert state['native_session']['home_input']=='0'
    else:assert state['native_tools']['error'] is None,(label,state['native_tools']['error'])
for label in ['initial','selected','zoomed','goto','typed_x','large','small','setup','start_six','step_quarter','zoom_three','custom_setup','custom_empty','custom_rows','sorted','home_restored']:
    assert states[label]['native_session']['home_input']=='12+',label
assert table('initial')[:2]==[[0,1,2,0],[.1,1.01,2.1,.0998334166468]]
assert numeric('selected')['row']==2
assert settings('zoomed')['start']=='0.1' and settings('zoomed')['step']=='0.05'
assert numeric('zoomed')['row']==2
assert settings('goto')['start']=='4.75' and numeric('goto')['row']==5
assert table('goto')[5][0]==5
assert settings('typed_x')['start']=='7.45' and numeric('typed_x')['row']==1
assert table('typed_x')[1][:3]==[7.5,57.25,9.5]
assert numeric('large')['font']==2 and numeric('small')['font']==0
assert settings('start_six')['start']=='6'
assert settings('invalid_step')['step']=='0.05'
assert settings('step_quarter')['step']=='0.25'
assert settings('zoom_three')['zoom']=='3' and settings('custom_setup')['kind']==1
assert table('custom_empty')==[]
assert [r[:3] for r in table('custom_rows')]==[[2,5,4],[4,17,6]]
assert [r[:3] for r in table('sorted')]==[[4,17,6],[2,5,4]]
for label in ['swap_cancel_form','swap_cancel_fields','swap_cancelled','swap_form','swap_fields']:
    assert [r[:3] for r in table(label)]==[[4,17,6],[2,5,4]],label
assert [r[:3] for r in table('swapped')]==[[2,5,4],[4,17,6]]
assert numeric('selection')['anchor']==[0,0] and numeric('selection')['column']==1 and numeric('selection')['row']==1
assert numeric('selection')['selecting'] and not numeric('selection_off')['selecting']
assert states['ppl_settings']['native_session']['last_outcome']['result']=='{6,0.25,3,1,{4,2}}'
for label,state in restored.items():
    assert state['native_tools']['error'] is None,label
    assert state['native_tools']['memory']['function_numeric']==numeric('final'),label
    assert state['native_tools']['memory']['graphs']['Function']==graph('final'),label
    assert state['calculator_app_numeric_settings']['Function']==settings('final'),label
assert [r[:3] for r in restored['reopened']['native_tools']['table']]==[[4,17,6],[2,5,4]]
# Pixel geometry is measured in calculator coordinates, independent of window DPI.
def pixel(label,x,y):
    state=states[label];rect=state['lcd_rect'];a,b=rect['min'],rect['max']
    path=root/f'numeric_v21_{label}.png';im=Image.open(path).convert('RGB')
    return im.getpixel((int(a[0]+(x+.5)*(b[0]-a[0])/320),int(a[1]+(y+.5)*(b[1]-a[1])/240)))
assert pixel('initial',40,45)==(173,204,255)
assert pixel('selected',40,80)==(173,204,255)
assert pixel('initial',130,25)==(232,241,246)
assert pixel('initial',130,45)==(255,255,255)
assert pixel('goto_form',310,55)==(255,255,255)
assert pixel('goto_form',40,100)==(232,241,246)
print('Function Numeric: measured grid/fonts/forms, real zoom and X navigation, async invalid-input recovery, custom rows/sort, actual PPL settings, Home preservation and independent restart passed')
