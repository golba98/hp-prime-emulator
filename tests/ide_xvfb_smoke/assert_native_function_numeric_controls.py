"""Real Numeric menu navigation, pinned columns, clipboard and Plot confirmation."""
import json
import math
import sys
from pathlib import Path
from PIL import Image
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
report=json.loads((root/'native_side_function_numeric_controls.json').read_text())
assert not report['wait_timeouts'],report['wait_timeouts']
states={s['label']:s['state'] for s in report['snapshots']}
for label,state in states.items():
    assert state['native_tools']['error'] is None,(label,state['native_tools']['error'])
    assert not state['worker_alive'] and not state['native_tools']['calculating'],label
def settings(label):return states[label]['calculator_app_numeric_settings']['Function']
def values(label):return states[label]['native_numeric_values']
assert values('empty')[:2]==[[{'type':'real','value':'0'}],[{'type':'real','value':'0.1'}]]
assert [v['value'] for v in values('five')[0]]==['0','0','1','2','3','4']
assert states['horizontal']['native_tools']['memory']['function_numeric']['horizontal_scroll']==40
assert states['horizontal']['native_tools']['memory']['function_numeric']['column']==5
assert states['row_paste']['native_session']['home_input']=='{{0,0,1,2,3,4}}'
for label,start,step in [('in','0.1','0.05'),('out','0','0.1'),('undo','0.1','0.05'),('decimal','0','0.1'),('integer','0','1'),('gradians','0','7.5'),('degrees','0','7.5')]:
    assert (settings(label)['start'],settings(label)['step'])==(start,step),(label,settings(label))
assert settings('radians')['step']=='0.1308996939'
for label in ['plot_confirmation','plot_cancelled']:
    assert settings(label)==settings('setup'),label
window=states['plot_accepted']['native_tools']['memory']['graphs']['Function']['window']
assert float(settings('plot_accepted')['start'])==window['xmin']
assert math.isclose(float(settings('plot_accepted')['step']),(window['xmax']-window['xmin'])/318,abs_tol=1E-12)
assert settings('plot_table')==settings('plot_accepted')
# Screen checks use calculator coordinates at the actual displayed LCD scale.
def pixel(label,x,y):
    a,b=states[label]['lcd_rect']['min'],states[label]['lcd_rect']['max']
    im=Image.open(root/f'numeric_controls_{label}.png').convert('RGB')
    return im.getpixel((int(a[0]+(x+.5)*(b[0]-a[0])/320),int(a[1]+(y+.5)*(b[1]-a[1])/240)))
assert pixel('empty',150,70) in [(173,204,255),(197,220,252)]
assert pixel('horizontal',20,45)==(255,255,255)
assert pixel('horizontal',300,45)==(173,204,255)
assert pixel('plot_confirmation',80,87)==(255,255,255)
assert pixel('plot_cancelled',80,87)==(232,241,246)
print('Function Numeric controls: actual five-column scrolling, row clipboard, Zoom/Un-Zoom/Decimal/Integer/Trig, independent angle and confirmed Plot copy passed')
