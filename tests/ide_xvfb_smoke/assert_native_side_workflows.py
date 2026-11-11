#!/usr/bin/env python3
"""Assert real desktop calculator workflows, including separate-process reopen."""
import json
from pathlib import Path
from PIL import Image
ROOT=Path('/tmp/ppl-ide-e2e')
checks=0

def check(value,detail):
    global checks
    assert value,detail
    checks+=1

def report(name):
    data=json.loads((ROOT/f'native_side_{name}.json').read_text())
    check(not data['wait_timeouts'],(name,data['wait_timeouts']))
    return data,{entry['label']:entry['state'] for entry in data['snapshots']}

_,states=report('state_create')
for label in ['stored_catalog','closed_tab_catalog']:
    catalog=states[label]['native_session']['catalog']
    check(any(entry['name']=='SAVED' for entry in catalog),catalog)
check(states['stored_run']['worker_outcome']['value']=='7',states['stored_run']['worker_outcome'])
check(states['cas_answer']['calculator_cas']['last_value']=={'type':'integer','value':'12'},states['cas_answer']['calculator_cas'])
_,states=report('state_reopen')
reopened=states['reopened']
check(reopened['calculator_memory']['A']['value']=='2',reopened['calculator_memory'])
check([v['value'] for v in reopened['calculator_memory']['L1']['value']]==['1','2','3'],'restored list')
check(reopened['calculator_memory']['M1']['rows']==2 and reopened['calculator_memory']['M1']['cols']==2,'restored matrix shape')
check([v['value'] for v in reopened['calculator_memory']['M1']['value']]==['1','2','3','4'],'restored matrix values')
check(reopened['calculator_notes']['NOTE']=='Retained calculator note','restored note')
check('SAVED' in reopened['calculator_programs'],'restored installed program')
check(reopened['native_tools']['memory']['app_notes']['Function']=='Retained app information','separate app Info note survives restart')
check(states['restored_values']['native_session']['last_outcome']['result']=='5','restored value arithmetic')
check(states['restored_cas']['calculator_cas']['last_value']=={'type':'integer','value':'36'},states['restored_cas']['calculator_cas'])
check(states['restored_program']['worker_outcome']['value']=='7','restored installed execution')
_,states=report('graph')
check(states['definition']['native_tools']['memory']['graphs']['Function']['curves'][0]['expression']=='X^2','actual curve entry')
points=states['graph']['native_tools']['plot_points']
check(bool(points) and len(points[0])>=300,'actual plot samples')
check(all(p is not None and abs(p[1]-p[0]**2)<1e-8 for p in points[0]),'sampled parabola')
check(states['numeric']['native_tools']['table'][:5]==[[0,0],[1,1],[2,4],[3,9],[4,16]],'numeric view shares definition')
trace=states['trace_first']['native_plot_trace']
next_trace=states['trace_next']['native_plot_trace']
check(trace['enabled'] and trace['curve']==0,'plot arrows enable trace on actual curve')
check(next_trace['sample']==trace['sample']+1,'plot arrows navigate actual samples')
window=states['zoomed']['native_tools']['memory']['graphs']['Function']['window']
original=states['graph']['native_tools']['memory']['graphs']['Function']['window']
check(abs(window['xmax']-window['xmin']-(original['xmax']-original['xmin'])/2)<1e-10,'Plus zoom changes plot window')
check(states['zoomed']['native_tools']['plot_points'][0][0][0]==window['xmin'],'zoom resamples actual expression')

image=Image.open(ROOT/'native_side_graph_lcd.png').convert('RGB')
check(image.size==(320,240),'composed LCD size')
check(sum(b>150 and r<100 and g<120 for r,g,b in zip(image.tobytes()[0::3],image.tobytes()[1::3],image.tobytes()[2::3]))>80,'actual visible blue parabola in saved tool LCD')
_,states=report('dialogs')
result=states['dialog_result']['native_session']['last_outcome']
check(result['status']=='success',result)
check(result['typed_result']['value'][0]['value']=='6','INPUT physical keypad update')
for matrix in result['typed_result']['value'][1:]:
    check((matrix['rows'],matrix['cols'])==(3,2),'EDITMAT variable and return shape')
    check([v['value'] for v in matrix['value']]==['0','0','9','2','3','4'],'EDITMAT touch inserted row and physical cell edit')
_,states=report('user')
for label,text,count,mode in [('temporary','7','1',''),('persistent','77','3','↑U')]:
    state=states[label]
    check(state['native_session']['home_input']==text,'KEY return inserts raw text')
    check(state['native_session']['user_mode']==mode,'User activation/consumption')
    check(state['calculator_memory']['Ans']['value']=='5','KEY preserves Ans')
    check(state['calculator_memory']['A']['value']==count,'KEY shares calculator state')
check(states['evaluated']['native_session']['last_outcome']['result']=='77','inserted text remains editable/evaluable')
_,states=report('integer')
for label,value in [('base_open',8),('base_shift',16),('base_saved',16)]:
    check(states[label]['native_tools']['integer_edit']['raw']==value,(label,'integer editing'))
    check(states[label]['calculator_memory']['Ans']['value']=='8','Edit Integer never rewrites history answer')
check(states['program_management']['native_tools']['page']=='Programs','Shift1 opens actual management catalog')
_,states=report('clipboard')
for label,text in [('deleted',''),('system_paste','12345'),('system_cut',''),('system_cut_paste','12345')]:
    check(states[label]['native_session']['home_input']==text,(label,'actual OS clipboard path'))
_,states=report('models')
payment=states['payment']['native_model_results']['Finance']
check(abs(payment+315.1691758344075)<1e-9,'documented finance payment calculation')
check(abs(states['payment']['native_tools']['memory']['models']['finance']['payment']-payment)<1e-12,'solved PMT updates app field')
plot=states['amortization_plot']['native_model_results']['Finance:plot']
check(len(plot['curves'])==3 and bool(plot['curves'][0]),'actual amortization curves')
schedule=states['amortization_numeric']['native_model_results']['Finance:amortization']
check(len(schedule['groups'])==60,'amortization numerical schedule')
check(abs(schedule['terminal_balance'])<0.2,'amortization closes loan within cent rounding')

_,states=report('solve')
check(states['equation']['native_tools']['memory']['models']['equation']=='x^2=A','Solve actual equation field')
check(states['solutions']['native_model_results']['Solve']=={'type':'list','value':[{'type':'integer','value':'-2'},{'type':'integer','value':'2'}]},'Solve uses Home variable A=4')
plot=states['equation_plot']['native_model_results']['Solve:plot']
check(len(plot['curves'])==2,'Solve plots both equation sides')
check(all(abs(y-x*x)<1e-8 for x,y in plot['curves'][0]),'Solve left curve evaluates expression')
check(all(y==4 for x,y in plot['curves'][1]),'Solve right curve uses same Home memory')
check(states['equation_plot']['native_tools']['error'] is None,'Solve graph completed without hidden error')

_,states=report('graph_context')
state=states['shared_graph']
check(state['native_tools']['error'] is None,('graph variable worker',state['native_tools']['error']))
points=state['native_tools']['plot_points']
check(bool(points) and len(points[0])>=300,'installed function actually sampled')
check(all(p is not None and abs(p[1]-p[0]-11)<1e-8 for p in points[0]),'graph function shares Home T and reachable CAS q')
check(states['shared_table']['native_tools']['table'][:3]==[[0,11],[1,12],[2,13]],'graph/table share installed function and memory')
check(state['calculator_memory']['T']['value']=='7','graph independent parameter preserves Home T')
check(state['calculator_cas']['bindings']['q']=='4','graph resolution preserves CAS object')
print(f'Native side workflows: {checks} behavioral assertions passed')
