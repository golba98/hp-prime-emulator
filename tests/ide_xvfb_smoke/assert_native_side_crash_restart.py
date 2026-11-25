#!/usr/bin/env python3
"""Kill an actual GUI after durable writes, then open a separate GUI process."""
import json
import os
import signal
import subprocess
import tempfile
import time
from pathlib import Path
HERE=Path(__file__).resolve().parent
ROOT=HERE.parent.parent
out=Path(tempfile.mkdtemp(prefix='prime-native-crash-'))
script=json.loads((HERE/'scripts/script_native_side_state_create.json').read_text())
script['steps'][4:4]=[
    {'step':'native_object','kind':'variable','name':'B','content':'1°22′30″'},
    {'step':'wait_idle','timeout_ms':5000},
    {'step':'native_object','kind':'variable','name':'C','content':'2∡1'},
    {'step':'wait_idle','timeout_ms':5000},
    {'step':'native_object','kind':'variable','name':'L2','content':'{B,C}'},
    {'step':'wait_idle','timeout_ms':5000},
    {'step':'native_object','kind':'variable','name':'D','content':'EXPR("2_m")'},
    {'step':'wait_idle','timeout_ms':5000},
    {'step':'native_object','kind':'variable','name':'E','content':'CAS("[1,2]")'},
    {'step':'wait_idle','timeout_ms':5000},
    {'step':'native_object','kind':'variable','name':'F','content':'R→B(-5,-8)'},
    {'step':'wait_idle','timeout_ms':5000},
    {'step':'native_object','kind':'variable','name':'G','content':'SETBASE(R→B(255,-8),0)'},
    {'step':'wait_idle','timeout_ms':5000},
]
script['steps']=[step for step in script['steps'] if step['step']!='exit']+[{'step':'wait_ms','ms':30000}]
path=out/'create.json';path.write_text(json.dumps(script))
helper=Path(os.environ.get('PPL_CAS_HELPER',str(Path(os.environ.get('XDG_DATA_HOME',Path.home()/'.local/share'))/'hp-prime-emulator/cas/ppl-giac-helper')))
if helper.is_file():os.environ['PPL_CAS_HELPER']=str(helper)
pid_file = out / 'gui.pid'
env=dict(os.environ,PPL_IDE_NO_DEVICE='1',PPL_NATIVE_PROFILE=str(out/'memory'),XDG_CONFIG_HOME=str(out/'config'),XDG_DATA_HOME=str(out/'data'),PPL_IDE_LOG_DIR=str(out/'create-log'),PPL_IDE_PID_FILE=str(pid_file))
with (out/'runner.log').open('w') as log:
    process=subprocess.Popen(['bash',str(HERE/'run_ide_auto.sh'),str(path),'60'],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT)
    deadline=time.monotonic()+25
    try:
        while True:
            if process.poll() is not None:raise AssertionError('GUI exited before crash checkpoint')
            try:
                state=json.loads((out/'memory/state-v1.json').read_text())
                memory=state['memory'];ui=state['ui']
                ready=memory['cas']['last_value']=={'type':'integer','value':'12'} and 'SAVED' in memory['programs'] and memory['notes']['NOTE']=='Retained calculator note' and len(ui['tools']['cas_history'])>=3 and ui['tools']['app_notes'].get('Function')=='Retained app information' and memory['globals']['B']['presentation']=='sexagesimal' and memory['globals']['C']['presentation']=='polar' and memory['globals']['L2']['value'][1]['presentation']=='polar' and memory['globals']['D']['value']=={'type':'unit','value':'2_m'} and memory['globals']['E']['vector'] and memory['globals']['F'].get('negative') is True and memory['globals']['G'].get('signed_word') is True
            except (FileNotFoundError,json.JSONDecodeError,KeyError,TypeError):ready=False
            if ready:break
            if time.monotonic()>deadline:raise AssertionError('durable calculator objects never reached crash checkpoint')
            time.sleep(.03)
        pid = int(pid_file.read_text())
        assert pid > 1 and pid != process.pid and process.poll() is None, pid
        os.kill(pid, signal.SIGKILL)
        assert process.wait(timeout=10) == 128 + signal.SIGKILL, 'expected a SIGKILL exit'
    finally:
        if process.poll() is None:process.terminate();process.wait(timeout=10)
env['PPL_IDE_LOG_DIR']=str(out/'reopen-log')
env.pop('PPL_IDE_PID_FILE')
reopen_script=json.loads((HERE/'scripts/script_native_side_state_reopen.json').read_text())
reopen_script['steps'][-1:-1]=[
    {'step':'native_object','kind':'variable','name':'Base','content':'0'},
    {'step':'wait_idle','timeout_ms':5000},
    {'step':'native_object','kind':'variable','name':'H','content':'B→R(F)'},
    {'step':'wait_idle','timeout_ms':5000},
    {'step':'native_object','kind':'variable','name':'I','content':'STRING(G)'},
    {'step':'wait_idle','timeout_ms':5000},
    {'step':'snapshot','label':'word_restored'},
]
reopen_path=out/'reopen.json';reopen_path.write_text(json.dumps(reopen_script))
subprocess.run(['bash',str(HERE/'run_ide_auto.sh'),str(reopen_path),'45'],cwd=ROOT,env=env,check=True)
data=json.loads(Path('/tmp/ppl-ide-e2e/native_side_state_reopen.json').read_text())
assert data['wait_timeouts']==[],data['wait_timeouts']
states={entry['label']:entry['state'] for entry in data['snapshots']}
reopened=states['reopened'];assert reopened['calculator_notes']['NOTE']=='Retained calculator note'
assert 'SAVED' in reopened['calculator_programs']
assert reopened['native_tools']['memory']['app_notes']['Function']=='Retained app information'
assert states['restored_values']['native_session']['last_outcome']['result']=='5'
assert states['restored_cas']['calculator_cas']['last_value']=={'type':'integer','value':'36'}
assert states['restored_program']['worker_outcome']['value']=='7'
print(f'Forced-termination desktop restart: 8 assertions passed; evidence {out}')
assert reopened['calculator_memory']['B']['presentation']=='sexagesimal'
assert reopened['calculator_memory']['C']['presentation']=='polar'
assert reopened['calculator_memory']['L2']['value'][0]['presentation']=='sexagesimal'
assert reopened['calculator_memory']['L2']['value'][1]['presentation']=='polar'
assert reopened['calculator_memory']['B']['value']=='1.375'
print('Forced-termination restart also retained real, complex and collection presentation metadata: 5 assertions passed')
assert reopened['calculator_memory']['D']=={'type':'symbolic','value':{'type':'unit','value':'2_m'}}
assert reopened['calculator_memory']['E']['vector']
assert [value['value'] for value in reopened['calculator_memory']['E']['value']]==['1','2']
print('Forced-termination restart also retained the typed unit and numeric CAS vector: 3 assertions passed')

words=states['word_restored']['calculator_memory']
durable=json.loads((out/'memory/state-v1.json').read_text())['memory']['globals']
assert words['F']['type']=='integer' and durable['F']['signed_word'] and durable['F']['bits']==8 and durable['F']['negative'] and durable['F']['value']=='251' and durable['F']['system_base']
assert words['G']['type']=='integer' and durable['G']['signed_word'] and durable['G']['bits']==8 and not durable['G'].get('negative',False) and durable['G']['value']=='255' and durable['G']['system_base']
assert words['H']=={'type':'real','value':'-5'}
assert words['I']=={'type':'string','value':'#11111111:-8b'}
print('Forced-termination restart retained signed word values, widths and system-base affinity; real conversion and current-base formatting passed')
