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
script['steps']=[step for step in script['steps'] if step['step']!='exit']+[{'step':'wait_ms','ms':30000}]
path=out/'create.json';path.write_text(json.dumps(script))
helper=Path(os.environ.get('PPL_CAS_HELPER',str(Path(os.environ.get('XDG_DATA_HOME',Path.home()/'.local/share'))/'hp-prime-emulator/cas/ppl-giac-helper')))
if helper.is_file():os.environ['PPL_CAS_HELPER']=str(helper)
env=dict(os.environ,PPL_IDE_NO_DEVICE='1',PPL_NATIVE_PROFILE=str(out/'memory'),XDG_CONFIG_HOME=str(out/'config'),XDG_DATA_HOME=str(out/'data'),PPL_IDE_LOG_DIR=str(out/'create-log'))
def descendants(pid):
    try: children=Path(f'/proc/{pid}/task/{pid}/children').read_text().split()
    except FileNotFoundError:return []
    found=[]
    for child in children:found.append(int(child));found.extend(descendants(int(child)))
    return found
with (out/'runner.log').open('w') as log:
    process=subprocess.Popen(['bash',str(HERE/'run_ide_auto.sh'),str(path),'60'],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT)
    deadline=time.monotonic()+25
    try:
        while True:
            if process.poll() is not None:raise AssertionError('GUI exited before crash checkpoint')
            try:
                state=json.loads((out/'memory/state-v1.json').read_text())
                memory=state['memory'];ui=state['ui']
                ready=memory['cas']['last_value']=={'type':'integer','value':'12'} and 'SAVED' in memory['programs'] and memory['notes']['NOTE']=='Retained calculator note' and len(ui['tools']['cas_history'])>=3 and ui['tools']['app_notes'].get('Function')=='Retained app information'
            except (FileNotFoundError,json.JSONDecodeError,KeyError,TypeError):ready=False
            if ready:break
            if time.monotonic()>deadline:raise AssertionError('durable calculator objects never reached crash checkpoint')
            time.sleep(.03)
        pids=[pid for pid in descendants(process.pid) if Path(f'/proc/{pid}/comm').read_text().strip()=='ppl']
        assert len(pids)==1,pids
        os.kill(pids[0],signal.SIGKILL)
        assert process.wait(timeout=10)!=0,'crash must not be a graceful GUI exit'
    finally:
        if process.poll() is None:process.terminate();process.wait(timeout=10)
env['PPL_IDE_LOG_DIR']=str(out/'reopen-log')
subprocess.run(['bash',str(HERE/'run_ide_auto.sh'),str(HERE/'scripts/script_native_side_state_reopen.json'),'45'],cwd=ROOT,env=env,check=True)
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
