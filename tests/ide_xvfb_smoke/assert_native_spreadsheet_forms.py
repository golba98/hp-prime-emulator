"""Prime worksheet input forms, retained invalid drafts and Auto dimensions."""
import json
import sys
from pathlib import Path
from PIL import Image
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
created=json.loads((root/'native_side_spreadsheet_forms.json').read_text())
reopened=json.loads((root/'native_side_spreadsheet_forms_reopen.json').read_text())
states={s['label']:s['state'] for s in created['snapshots']}
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
for label,s in states.items():
    assert s['native_tools']['page']=='Numeric',label
    assert not s['native_tools']['calculating'],label
    if label not in ['goto_invalid','width_invalid']:assert s['native_tools']['error'] is None,(label,s['native_tools']['error'])
    assert s['calculator_memory']['Ans']['value']=='7'
assert states['goto_empty']['native_session']['home_input']==''
assert states['goto_invalid']['native_session']['home_input']=='ZZ10001'
assert states['goto_invalid']['native_tools']['error']
cursor=states['goto_range']['native_tools']['memory']['spreadsheet_cursor']
assert cursor['anchor']==[2,2] and cursor['selecting']
assert states['goto_range']['native_tools']['memory']['models']['selected_cell']=='A1'
assert states['name_empty']['native_session']['home_input']==''
assert states['width_auto']['native_session']['home_input']=='-1'
assert states['height_auto']['native_session']['home_input']=='-1'
assert states['width_invalid']['native_session']['home_input']=='9999'
assert states['width_invalid']['native_tools']['error']
assert not states['width_invalid']['native_tools']['memory']['models']['spreadsheet'].get('column_widths')
assert states['width_current']['native_session']['home_input']=='80'
for document in [created,reopened]:
    s=document['snapshots'][-1]['state'];sheet=s['native_tools']['memory']['models']['spreadsheet']
    assert sheet['cells']=={'A1':'5'}
    assert sheet['names']['ORIGIN']==dict(left=1,right=1,top=1,bottom=1)
    assert not sheet.get('column_widths') and sheet['row_heights']=={'2':25}
    assert s['native_model_results']['Spreadsheet']['A1']['Number']==5
    assert s['native_tools']['memory']['models']['selected_cell']=='A1'
    assert s['native_tools']['error'] is None
def pixel(label,x,y):
    rect=states[label]['lcd_rect'];left,top=rect['min'];right,bottom=rect['max']
    return Image.open(root/f'spreadsheet_forms_{label}.png').convert('RGB').getpixel((round(left+x*(right-left)/320),round(top+y*(bottom-top)/240)))
paper=pixel('goto_empty',30,60)
assert max(paper)-min(paper)<40 and min(paper)>210
assert min(pixel('goto_empty',200,100))>235
assert max(pixel('goto_empty',200,88))<80
assert pixel('goto_range',40,45)!=pixel('goto_range',100,45)
print('Spreadsheet forms: Prime field layout,range selection,retained invalid drafts/correction,names,Auto−1 dimensions/reset and separate-process persistence passed')
