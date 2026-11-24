#!/usr/bin/env python3
"""Actual desktop root-field navigation, evaluated roots and symbolic CAS Show."""
import json
from pathlib import Path

d = json.loads(Path('/tmp/ppl-ide-e2e/native_side_textbook.json').read_text())
assert d['wait_timeouts'] == [], d['wait_timeouts']
n = {s['label']: s['state']['native_session'] for s in d['snapshots']}
assert n['nthroot_degree_down_stays']['home_input'] == 'NTHROOT(3,)'
assert n['nthroot_degree_down_stays']['cursor'] == 9
assert n['nthroot_degree_down_stays']['history_position'] is None
assert n['nthroot_entry']['home_input'] == 'NTHROOT(3,8)'
assert n['nthroot_up_stays_in_body']['home_input'] == 'NTHROOT(3,8)'
assert n['nthroot_up_stays_in_body']['cursor'] == 11
assert n['nthroot_radicand']['cursor'] == 11
assert n['nthroot_radicand']['history_position'] is None
assert n['nthroot_pointer_body']['cursor'] == 10
assert n['nthroot_pointer_body']['home_input'] == 'NTHROOT(3,8)'
assert n['nthroot_pointer_edited']['home_input'] == 'NTHROOT(3,27)'
assert n['nthroot_result']['history'][-1]['result'] == '3'
assert n['cas_rational']['history'][-1]['typed_result'] == {'type': 'symbolic', 'value': '(x^2+1)/(2*x)'}
assert n['cas_rational']['history'][-1]['error'] is False
assert n['cas_show']['history_viewer'] == {'x': 0, 'y': 0}
assert n['cas_show']['run_id'] == n['cas_rational']['run_id']
assert n['home_restored']['history'][-1]['result'] == '3'
assert len(n['home_restored']['history']) == 1
assert n['fraction_entry']['home_input'] == '(1)/(2)'
assert n['fraction_numerator']['home_input'] == '(3)/(2)'
assert n['fraction_numerator']['cursor'] == 2
assert n['fraction_denominator']['cursor'] == 6
assert n['fraction_result']['history'][-1]['result'] == '1.5'
print('Native textbook: nth-root field navigation/result, real Giac rational Show and Home/CAS histories passed')
