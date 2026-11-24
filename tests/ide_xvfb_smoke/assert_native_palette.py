#!/usr/bin/env python3
"""All 18 G1 palette cells: real pointer selection, field keys and computed values."""
import json
import sys
from pathlib import Path
from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')
d = json.loads((root / 'native_side_palette.json').read_text())
assert d['wait_timeouts'] == [], d['wait_timeouts']
s = {x['label']: x['state'] for x in d['snapshots']}
cases = [
    ('fraction', '(1)/(2)', '0.5'),
    ('power', '(2)^(3)', '8'),
    ('index', '(L1)[2]', '11'),
    ('derivative', 'CAS.diff(x^2,x)', '2*x'),
    ('piecewise', 'CAS.piecewise(1>0,2,1<0,3)', '2'),
    ('matrix', '[[1,2],[3,4]]', '[[1,2],[3,4]]'),
    ('root', 'SQRT(9)', '3'),
    ('nthroot', 'NTHROOT(3,8)', '2'),
    ('limit', 'CAS.limit(SIN(x)/x,x,0,)', '1'),
    ('integral', 'CAS.integrate(x,x,0,1)', '1/2'),
    ('row_vector', '[2,3]', '[2,3]'),
    ('column_vector', '[[2],[3]]', '[[2],[3]]'),
    ('abs', 'ABS(-2)', '2'),
    ('mixed', '((2)+(1)/(3))', '2.33333333333'),
    ('sum', 'CAS.sum(k,k,1,5)', '15'),
    ('product', 'CAS.product(k,k,1,4)', '24'),
    ('logbase', 'LOG(8,2)', '3'),
    ('sexagesimal', '→HMS((12)+(30)/60+(0)/3600)', '12°30′00″'),
]
for i, (name, source, result) in enumerate(cases):
    entry = s[name + '_entry']['native_session']
    after = s[name + '_result']['native_session']
    assert entry['home_input'] == source, (name, entry['home_input'])
    assert len(after['history']) == i + 2, (name, after['history'])
    assert after['home_input'] == '', (name, after['home_input'])
    assert after['last_outcome']['status'] == 'success', (name, after['last_outcome'])
    assert after['history'][-1]['expression'] == source, name
    assert after['history'][-1]['result'] == result, (name, after['history'][-1])
    assert not after['history'][-1]['error'], name
    assert s[name + '_empty']['native_tools']['page'] is None
    for suffix in ['_entry', '_result']:
        image = Image.open(root / ('native_palette_' + name + suffix + '.png'))
        assert image.size == (320, 240), (name, suffix, image.size)
for name, dims in [('matrix', (2, 2, False)), ('row_vector', (1, 2, True)), ('column_vector', (2, 1, False))]:
    typed = s[name + '_result']['native_session']['history'][-1]['typed_result']
    assert (typed['rows'], typed['cols'], typed['vector']) == dims
assert s['sexagesimal_result']['native_session']['history'][-1]['typed_result']['value'] == '12.5'
cas = s['cas_integral_result']['native_session']
assert len(cas['history']) == 1
assert cas['history'][-1]['expression'] == 'CAS.integrate(x,x,0,1)'
assert cas['history'][-1]['result'] == '1/2'
assert not cas['history'][-1]['error']
assert s['cas_palette_reset']['native_tools']['memory']['template_selection'] == 'Fraction'
assert s['cas_palette_dismissed']['native_tools']['page'] is None
assert s['cas_palette_dismissed']['native_session']['history'] == cas['history']
assert len(s['home_history_restored']['native_session']['history']) == 19
assert s['dms_recalled']['native_session']['home_input'] == cases[-1][1]
assert s['dms_recalled_result']['native_session']['history'][-1]['expression'] == '→HMS((12)+(30)/60+(01)/3600)'
assert s['dms_recalled_result']['native_session']['history'][-1]['result'] == '12°30′01″'
# G1 palette box, selection and separators measured from the genuine firmware.
grid = Image.open(root / 'native_palette_grid.png').convert('RGB')
assert grid.size == (320, 240)
assert grid.getpixel((46, 66)) == (173, 204, 255)
assert grid.getpixel((273, 174)) == (232, 241, 246)
assert max(grid.getpixel((45, 65))) < 100
separator = grid.getpixel((100, 102))
assert all(215 <= channel <= 228 for channel in separator) and max(separator) - min(separator) <= 3
print('18 Prime palette forms: actual grid clicks, natural field navigation, results/types, CAS outline, recall and measured palette geometry passed')
