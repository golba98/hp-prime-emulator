#!/usr/bin/env python3
"""Force-kill the actual Statistics GUI, then verify a separate-process restore."""
import hashlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
base = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else None
if base is not None:
    base.mkdir(parents=True, exist_ok=True)
out = Path(tempfile.mkdtemp(prefix='prime-statistics-crash-', dir=base))
script = json.loads((HERE / 'scripts/script_native_side_statistics_one_charts.json').read_text())
script['dump'] = str(out / 'create-dump.json')
script['steps'] = [step for step in script['steps'] if step['step'] not in ['exit', 'window_screenshot']]
script['steps'].extend([
    {'step': 'native_object', 'kind': 'variable', 'name': 'D2',
     'content': '{1234567890123456789,1/3,1E400,1E-400}'},
    {'step': 'wait_idle', 'timeout_ms': 15000},
    {'step': 'wait_ms', 'ms': 200},
])
script['steps'].append({'step': 'wait_ms', 'ms': 30000})
create = out / 'create.json'
create.write_text(json.dumps(script, indent=2))
helper = Path(os.environ.get('PPL_CAS_HELPER', str(Path(os.environ.get('PPL_BIN', 'target/debug/ppl')).parent / 'ppl-giac-helper')))
pid_file = out / 'gui.pid'
env = dict(os.environ, PPL_IDE_NO_DEVICE='1', PPL_CONFIG='/dev/null',
           PPL_NATIVE_PROFILE=str(out / 'profile'), XDG_CONFIG_HOME=str(out / 'config'),
           XDG_DATA_HOME=str(out / 'data'), PPL_IDE_LOG_DIR=str(out / 'create-log'),
           PPL_IDE_PID_FILE=str(pid_file))
if helper.is_file():
    env['PPL_CAS_HELPER'] = str(helper.resolve())
binaries = {}
for label, binary in [('ppl', Path(env.get('PPL_BIN', 'target/debug/ppl'))), ('giac-helper', helper)]:
    if binary.is_file():
        digest = hashlib.sha256()
        with binary.open('rb') as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                digest.update(chunk)
        binaries[label] = {'path': str(binary.resolve()), 'sha256': digest.hexdigest()}
(out / 'binaries.json').write_text(json.dumps(binaries, indent=2))
for name in ['PPL_PRIME_FONT_RESOURCE', 'PPL_FACTORY_A', 'PPL_FIRMWARE_INIT_DATA']:
    env.pop(name, None)


def ready(state):
    book = state['ui']['tools']['models']['statistics1_book']
    analysis = book['analyses'][0]
    data = state['memory']['globals']['D2']['value']
    return (analysis['chart'] == 'pie' and analysis['enabled']
            and analysis['data_source'] == 'D1+1' and analysis['frequency_source'] == '2'
            and analysis['color'] == 0xff0000 and book['result_analysis'] == 0
            and book['result_values']['H1']['NbItem']['value'] == '5'
            and book['result_values']['H1']['ΣX']['value'] == '28'
            and book['result_values']['H1']['MeanX']['value'] == '5.6'
            and data[0]['origin']['numerator'] == '1234567890123456789'
            and data[1]['origin']['numerator'] == '1' and data[1]['origin']['denominator'] == '3'
            and data[2]['value'] == '1E400' and data[3]['value'] == '1E-400')


with (out / 'runner.log').open('w') as log:
    process = subprocess.Popen(['bash', str(HERE / 'run_ide_auto.sh'), str(create), '110'],
                               cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
    deadline = time.monotonic() + 80
    try:
        while True:
            assert process.poll() is None, 'GUI exited before durable crash checkpoint'
            try:
                durable = json.loads((out / 'profile/state-v1.json').read_text())
                complete = ready(durable)
            except (FileNotFoundError, json.JSONDecodeError, KeyError, TypeError):
                complete = False
            if complete:
                break
            assert time.monotonic() < deadline, 'Statistics state never reached durable crash checkpoint'
            time.sleep(.03)
        (out / 'before-kill.json').write_text(json.dumps(durable, indent=2))
        pid = int(pid_file.read_text())
        assert pid > 1 and pid != process.pid and process.poll() is None, pid
        os.kill(pid, signal.SIGKILL)
        assert process.wait(timeout=10) == 128 + signal.SIGKILL, 'expected a SIGKILL exit'
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=10)

reopen = json.loads((HERE / 'scripts/script_native_side_statistics_one_charts_reopen.json').read_text())
reopen['dump'] = str(out / 'reopened.json')
for step in reopen['steps']:
    if step['step'] == 'window_screenshot':
        step['path'] = str(out / Path(step['path']).name)
reopen_path = out / 'reopen.json'
reopen_path.write_text(json.dumps(reopen, indent=2))
env['PPL_IDE_LOG_DIR'] = str(out / 'reopen-log')
env.pop('PPL_IDE_PID_FILE')
subprocess.run(['bash', str(HERE / 'run_ide_auto.sh'), str(reopen_path), '45'],
               cwd=ROOT, env=env, check=True)
report = json.loads((out / 'reopened.json').read_text())
assert not report['wait_timeouts'], report['wait_timeouts']
states = {item['label']: item['state'] for item in report['snapshots']}
for label, state in states.items():
    assert state['native_tools']['error'] is None, (label, state['native_tools']['error'])
    assert not state['native_tools']['calculating'], label
before_book = durable['ui']['tools']['models']['statistics1_book']
restored_book = states['reopened']['native_tools']['memory']['models']['statistics1_book']
for field in ['analyses', 'result_values', 'result_analysis']:
    assert restored_book[field] == before_book[field], field
for name in ['D1', 'D2']:
    expected = durable['memory']['globals'][name]['value']
    actual = states['reopened']['calculator_memory'][name]['value']
    assert len(actual) == len(expected), name
    for want, value in zip(expected, actual):
        assert value['type'] == want['kind'] == 'real'
        assert value['value'] == want['value'], (name, value, want)
        assert value['origin'] == want['origin'], (name, value, want)
plot = states['reopened_plot']['native_model_results']['Statistics 1Var:plot']
assert len(plot['fills']) == 5 and len(plot['curves']) == 5
window = states['reopened_plot']['native_tools']['memory']['model_windows']['Statistics 1Var']
assert window == durable['ui']['tools']['model_windows']['Statistics 1Var']
assert all(window['xmin'] <= x <= window['xmax'] and window['ymin'] <= y <= window['ymax']
           for curve in plot['curves'] for x, y in curve)
summary = states['reopened_summary']['native_model_results']['Statistics 1Var']['H1']
# Pie counts each data row while retaining the saved frequency expression.
assert (summary['count'], summary['sum'], summary['mean']) == (5, 28, 5.6)
assert states['reopened_summary']['native_tools']['memory']['models']['statistics1_book']['result_values'] == before_book['result_values']
print(f'Statistics SIGKILL/restart passed: exact19-digit/rational/extreme typed columns, definitions/color, result selection/cache, saved plot bounds and calculated Pie/summary; evidence {out}')
