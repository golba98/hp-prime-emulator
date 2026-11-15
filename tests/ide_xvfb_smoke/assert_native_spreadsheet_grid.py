"""Real worksheet grid, inline templates, formats, selection and durable results."""
import json
import sys
from pathlib import Path
from PIL import Image

root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/ppl-ide-e2e')
created=json.loads((root/'native_side_spreadsheet_grid.json').read_text())
reopened=json.loads((root/'native_side_spreadsheet_grid_reopen.json').read_text())
states={s['label']:s['state'] for s in created['snapshots']}
for label,state in states.items():
    assert state['native_tools']['error'] is None,(label,state['native_tools']['error'])
    assert not state['native_tools']['calculating'],label
assert not created['wait_timeouts'] and not reopened['wait_timeouts']
assert states['sin_inserted']['native_session']['home_input']=='SIN()'
assert states['sin_inserted']['native_session']['cursor']==4
assert states['sin_inserted']['native_tools']['page']=='Numeric'
assert states['sin_deleted']['native_session']['home_input']==''
assert states['inline_sin']['native_session']['home_input']=='SIN(90)'
assert states['inline_selection']['native_session']['selection']==dict(start=0,end=7)
assert states['absolute_draft']['native_session']['home_input']=='=$A$1*2'
assert states['absolute_draft']['native_tools']['page']=='Numeric'
selection=states['block_selected']['native_tools']['memory']['spreadsheet_cursor']
assert selection['anchor']==[2,1] and selection['selecting']
assert states['block_selected']['native_tools']['memory']['models']['selected_cell']=='C2'
final=states['final'];restored=reopened['snapshots'][-1]['state']
for state in [final,restored]:
    assert state['native_tools']['error'] is None
    memory=state['native_tools']['memory'];sheet=memory['models']['spreadsheet']
    assert sheet['cells']['A1']=='1' and sheet['cells']['B1']=='=SUM(A1:A2)'
    assert sheet['cells']['D1']=='=TOTAL+B1' and sheet['cells']['E1']=='=$A$1*2'
    assert sheet['names']['TOTAL']==dict(left=1,right=1,top=1,bottom=1)
    assert sheet['column_widths']=={'2':80} and sheet['row_heights']=={'2':25}
    styles=[r['style'] for r in sheet['formats']]
    assert any(s['number']==1 and s['digits']==6 for s in styles)
    assert any(s['color']==255 for s in styles)
    assert any(s['fill']==0xffff00 for s in styles)
    values=state['native_model_results']['Spreadsheet']
    for cell,n in [('A1',1),('A2',3),('B1',4),('C1',1),('D1',5),('E1',2)]:
        assert values[cell]['Number']==n,(cell,values[cell])
    assert state['calculator_memory']['Ans']['value']=='7'
    assert memory['models']['selected_cell']=='D1'
assert final['native_tools']['memory']['models']['spreadsheet']==restored['native_tools']['memory']['models']['spreadsheet']

def pixels(label,box):
    lcd=states[label]['lcd_rect'];x,y=lcd['min'];right,bottom=lcd['max']
    b=(round(x+box[0]*(right-x)/320),round(y+box[1]*(bottom-y)/240),round(x+box[2]*(right-x)/320),round(y+box[3]*(bottom-y)/240))
    raw=Image.open(root/f'spreadsheet_{label}.png').convert('RGB').crop(b).tobytes()
    return list(zip(raw[::3],raw[1::3],raw[2::3]))
assert sum(b>r+80 and b>g+40 for r,g,b in pixels('blue',(17,37,82,53)))>2
assert sum(r>200 and g>200 and b<80 for r,g,b in pixels('block_fill',(82,37,212,70)))>200
# Inline SIN editing retains visible grid lines and cells above the entry row.
assert sum(max(rgb)<70 for rgb in pixels('inline_sin',(0,20,320,203)))>250
print('Spreadsheet: real grid/inline SIN deletion and entry,absolute reference,block drag,formats/name/dimensions,cancelled edit,computed values and separate-process persistence passed')
