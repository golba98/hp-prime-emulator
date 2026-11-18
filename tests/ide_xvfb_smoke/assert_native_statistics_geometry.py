"""Exercise Geometry values used by Statistics 1Var through PPL and the desktop."""
import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def states(scenario):
    report = json.loads((root / f'native_side_{scenario}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {entry['label']: entry['state'] for entry in report['snapshots']}


def result(state):
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    return [float(value['value']) for value in outcome['typed_result']['value']]


creation = states('statistics_geometry')
reopen = states('statistics_geometry_reopen')
assert result(creation['initial']) == [11.6666666667, 20, 2.75]
assert result(creation['redrawn']) == [20, 40, 2.75]
assert result(reopen['reopened']) == [40, 2.75]

definition = creation['definition_eval']
assert definition['native_tools']['page'] == 'Symbolic'
assert definition['native_tools']['error'] is None, definition['native_tools']['error']
assert definition['native_model_results']['Statistics 1Var:definition'] == '{approx(10),approx(10),approx(20)}'

for state in (creation['summary'], reopen['reopened_summary']):
    assert state['native_tools']['page'] == 'Numeric'
    assert state['native_tools']['error'] is None, state['native_tools']['error']
    summary = state['native_model_results']['Statistics 1Var']['H1']
    assert summary['count'] == 40 and summary['mean'] == 2.75, summary
    assert 'GC' not in state['calculator_memory']

state = creation['autoscale']
assert state['native_tools']['page'] == 'Plot'
assert state['native_tools']['error'] is None, state['native_tools']['error']
plot = state['native_model_results']['Statistics 1Var:plot']
assert plot['curves'] == [[[1, 10], [2, 10], [4, 20]]]
assert plot['scale']['bounds'][3] > 20

trace = creation['trace']['native_tools']['statistics_trace']
assert trace['analysis'] == 1 and trace['sample']['frequency'] in [10, 20], trace
assert trace['sample']['y'] == trace['sample']['frequency'], trace
print('Geometry expressions in Statistics 1Var PPL, Numeric Stats, Symbolic Eval, Autoscale, Trace and restart passed')
