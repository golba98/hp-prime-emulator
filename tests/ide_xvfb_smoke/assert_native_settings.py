#!/usr/bin/env python3
"""Real settings pointer/keyboard edits, calculations, recall and font pixels."""
import hashlib
import json
import sys
from pathlib import Path
from PIL import Image
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
def read(name):
 d=json.loads((root/('native_side_'+name+'.json')).read_text())
 assert not d['wait_timeouts'],d['wait_timeouts']
 return {s['label']:s['state']['native_session'] for s in d['snapshots']}
s=read('settings')
def setting(label):return s[label]['home_settings']
assert setting('opened')['page']==1
assert setting('angle_focused')['menu'] is None
assert setting('angle_choices')['menu']['choices']==['Radians','Degrees','Gradians']
assert setting('degrees')['angle']=='Degrees'
assert setting('format_choices')['menu']['choices']==['Standard','Fixed','Scientific','Engineering','Floating','Rounded']
assert setting('fixed_two')['format']==1 and setting('fixed_two')['digits']==2
assert s['fixed_history']['history'][0]['result']=='123456.789'
assert s['fixed_history']['history'][0]['home_display']['text']=='123﹐456.79'
assert s['algebraic']['presentation']['entry']=='algebraic'
assert setting('complex_pair_allowed')['complex']==11
assert setting('complex_pair_disallowed')['complex']==10
assert len(setting('grouping_choices')['menu']['choices'])==11
assert setting('ungrouped')['separator']==3
assert setting('page_two')['page']==2
assert s['small_selected']['presentation']['font_size']=='small'
assert s['large_selected']['presentation']['font_size']=='large'
assert setting('large_selected')['font_pixels']==16
assert s['system_font_program']['last_outcome']['result']=='17'
assert s['canonical_copy']['home_input']=='123456.789'
assert s['copied_result']['history'][-1]['typed_result']['value']=='123456.789'
assert setting('invalid_bits')['integer_bits']==32 and setting('invalid_bits')['error']
assert setting('valid_bits')['integer_bits']==64
assert setting('signed_64')['integer_bits']==64 and not setting('signed_64')['integer_signed'] and setting('signed_64')['error']
assert setting('signed_63')['integer_bits']==63 and setting('signed_63')['integer_signed']
assert setting('signed_bits_64_rejected')['integer_bits']==63 and setting('signed_bits_64_rejected')['integer_signed'] and setting('signed_bits_64_rejected')['error']
assert s['final']['home_input']==''
assert s['angle_calculation']['history'][-1]['typed_result']['value']=='0.5'
assert [v['value'] for v in s['settings_codes']['history'][-1]['typed_result']['value']]==['1','0','2','3','10']
# Locate the complete independent glyph region without masking or raster fitting.
if '--prime' in sys.argv:
 controls=json.loads((Path(__file__).resolve().parents[1]/'fixtures/native-ui-settings-controls.json').read_text())
 for control in controls['controls']:
  label='small' if control['font_px']==12 else 'large'
  im=Image.open(root/f'native_settings_{label}_history.png').convert('RGB')
  width,height=control['tight_size'];expected=control['tight_rgb24_sha256']
  matches=[(x,y) for x in range(230,320-width+1) for y in range(22,200-height+1)
           if hashlib.sha256(im.crop((x,y,x+width,y+height)).tobytes()).hexdigest()==expected]
  assert matches,(label,control)
 if control['font_px']==16:
  program=Image.open(root/'native_settings_system_font.png').convert('RGB')
  assert any(hashlib.sha256(program.crop((x,y,x+width,y+height)).tobytes()).hexdigest()==expected
             for x in range(0,5) for y in range(0,15)), 'Home font selection did not reach TEXTOUT_P font0'

if (root/'native_side_settings_reopen.json').exists():
 r=read('settings_reopen')
 assert r['restored']['presentation']['font_size']=='large'
 assert setting('signed_63')['separator']==r['restored_settings']['home_settings']['separator']
 assert r['restored_settings']['home_settings']['complex']==10
 assert r['restored_settings']['home_settings']['integer_bits']==63
 assert r['restored_settings']['home_settings']['integer_signed']
 assert r['restored_settings']['home_settings']['page']==1
print('Home settings: real edits, retroactive number paint, canonical recall and persistence passed' + ('; two independent G1 font regions passed' if '--prime' in sys.argv else ''))
