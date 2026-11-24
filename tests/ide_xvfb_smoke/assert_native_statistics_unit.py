"""Check the measured low-weight Statistics 1Var summaries and PPL results."""
import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')


def states(suffix=''):
    report = json.loads((root / f'native_side_statistics_unit{suffix}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {entry['label']: entry['state'] for entry in report['snapshots']}


def result(state):
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    typed = outcome['typed_result']['value']
    return [float(item['value']) if item['type'] == 'real' else item['value'] for item in typed]


initial = states()
reopened = states('_reopen')
assert result(initial['unit_ppl']) == [1, 1, 0, '+Inf', '+Inf', 0, '+Inf']
for state in (initial['subtotal_ppl'], reopened['reopened_ppl']):
    assert result(state) == [0.5, 2, 1, 1, 1]
for state, count, deviation in (
    (initial['unit_summary'], 1, '+Inf'),
    (initial['subtotal_summary'], 0.5, 'NaN'),
    (reopened['reopened_summary'], 0.5, 'NaN'),
):
    assert state['native_tools']['page'] == 'Numeric'
    assert state['native_tools']['error'] is None, state['native_tools']['error']
    summary = state['native_model_results']['Statistics 1Var']['H1']
    assert summary['count'] == count and summary['mean'] == 2
    assert summary['population_standard_deviation'] == 1
    for name in ('sample_standard_deviation', 'standard_error'):
        assert summary[name] is None
        assert summary['_display'][name] == deviation

print('Low-weight Statistics 1Var PPL, numeric summaries and profile reopen passed')
