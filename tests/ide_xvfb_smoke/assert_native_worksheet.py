"""Actual pointer/keyboard worksheet edits, VM results and durable entered values."""
import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
created = json.loads((root / 'native_side_worksheet.json').read_text())
reopened = json.loads((root / 'native_side_worksheet_reopen.json').read_text())
states = {s['label']: s['state'] for s in created['snapshots']}
initial = states['initial']['native_model_results']['Spreadsheet']
assert initial['C1']['Number'] == 9
assert initial['C2']['Number'] == 1
assert initial['A4']['Number'] == initial['B4']['Number'] == 50
assert initial['Z101']['Number'] == 127
for document in (created, reopened):
    assert not document['wait_timeouts']
    assert document['native_tools']['error'] is None
    sheet = document['native_tools']['memory']['models']['spreadsheet']
    assert sheet['cells']['A4'] == '50' and sheet['cells']['B4'] == '=P*5'
    assert sheet['entered_values']['A4'] == {'kind': 'real', 'value': '50', 'origin': {'kind': 'rational', 'numerator': '50', 'denominator': '1', 'negative': False}}
    values = document['native_model_results']['Spreadsheet']
    assert values['A4']['Number'] == 50 and values['B4']['Number'] == 100
    assert values['C1']['Number'] == 9 and values['C3']['Number'] == 1
    assert values['C4']['Number'] == 131
    assert abs(values['C2']['Number'] - 0.987688340595) < 1E-12
    assert values['Z101']['Number'] == 127
    assert document['calculator_memory']['P']['value'] == '20'
    assert document['calculator_memory']['Ans']['value'] == '3'
    assert document['native_tools']['memory']['models']['selected_cell'] == 'A4'
assert created['native_model_results'] == reopened['native_model_results']
print('Native worksheet GUI: Home/app angles, ranges, far address, frozen50/retained100 and reopened state passed')
