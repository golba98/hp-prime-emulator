"""Actual Spreadsheet format inheritance, local PPL getters, matrix paint and restart."""
import json
import math
import sys
from fractions import Fraction
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/tmp/ppl-ide-e2e')

def read(name):
    report = json.loads((root / f'native_side_{name}.json').read_text())
    assert not report['wait_timeouts'], report['wait_timeouts']
    return {item['label']: item['state'] for item in report['snapshots']}

states = read('spreadsheet_attributes')
restored = read('spreadsheet_attributes_reopen')
for label, state in [*states.items(), *restored.items()]:
    assert state['native_tools']['error'] is None, (label, state['native_tools']['error'])
    assert not state['native_tools']['calculating'], label
    assert not state['native_session']['home_input'], label


def sheet(state):
    return state['native_tools']['memory']['models']['spreadsheet']


def typed(value):
    if value.get('kind', value.get('type')) == 'list':
        return [typed(item) for item in value['value']]
    origin = value.get('origin', {})
    if origin.get('kind') == 'rational':
        number = float(Fraction(int(origin['numerator']), int(origin['denominator'])))
        return -number if origin.get('negative') else number
    return float(value['value'])


def result(state):
    outcome = state['native_session']['last_outcome']
    assert outcome['status'] == 'success', outcome
    return typed(outcome['typed_result'])


for label, state in [('local_formats', states['local_formats']),
                     ('fixed_local_values', states['fixed_local_values']),
                     ('auto_local_values', states['auto_local_values']),
                     ('textbook_local_values', states['textbook_local_values']),
                     ('color_local_values', states['color_local_values']),
                     ('reopened_local_values', restored['reopened_local_values'])]:
    values = result(state)
    assert values[0] == ([2, -1, -1, 2] if label in ['local_formats', 'fixed_local_values'] else [-1, -1, -1, 2]), (label, values)
    assert values[1] == ([-1, -1, -1] if label in ['local_formats', 'fixed_local_values', 'auto_local_values'] else [0, 1, -1]), (label, values)
    assert values[2] == ([-1, 2147483903, 4261413119, -1] if label in ['color_local_values', 'reopened_local_values'] else [255, 2147483903, 4261413119, 16777216]), (label, values)
    assert values[3][0] == (2 if label in ['local_formats', 'fixed_local_values'] else -1), (label, values)
    assert all(math.isclose(value, 1 / 3, rel_tol=1e-11) for value in values[3][1:]), (label, values)
    assert values[4] == ([1, -1] if label == 'fixed_local_values' else [-1, -1]), (label, values)

# Requested width100/height50 retain their local values; the actual grid adds
# one pixel for each line, as independently measured on the original G1.
for state in [states['sized'], states['final'], restored['reopened_grid']]:
    saved = sheet(state)
    assert all(saved['column_widths'][str(col)] == 100 for col in [1, 2, 3])
    assert all(saved['row_heights'][str(row)] == 50 for row in [1, 2, 3, 4, 5])


def cell_pixels(label, address):
    state = (states if label in states else restored)[label]
    cursor = state['native_tools']['memory']['spreadsheet_cursor']
    column = ord(address[0]) - ord('A') + 1
    row = int(address[1:])
    x = 10 + (column - cursor['first_column']) * 101
    y = 37 + (row - cursor['first_row']) * 51
    assert 10 <= x < 313 and 37 <= y < 190, (label, address, cursor)
    rect = state['lcd_rect']
    left, top = rect['min']
    right, bottom = rect['max']
    # Measure font thresholds in LCD pixels. The full-keypad layout can
    # render the window at a fractional physical scale.
    columns = [math.floor(left + (col + 0.5) * (right - left) / 320)
               for col in range(x + 1, x + 98)]
    rows = [math.floor(top + (row + 0.5) * (bottom - top) / 240)
            for row in range(y + 2, y + 48)]
    with Image.open(root / f'spreadsheet_attributes_{label}.png') as capture:
        image = capture.convert('RGB')
        cell = Image.new('RGB', (len(columns), len(rows)))
        cell.putdata([image.getpixel((col, row)) for row in rows for col in columns])
    return cell


def ink_height(label, address, red=False):
    image = cell_pixels(label, address)
    rows = []
    for y in range(image.height):
        for x in range(image.width):
            r, g, b = image.getpixel((x, y))
            ink = r > 140 and g < 100 and b < 100 if red else max(r, g, b) < 100
            if ink:
                rows.append(y)
                break
    assert rows, (label, address, red)
    return max(rows) - min(rows) + 1


# Cell14 beats column22 and later row18. Column10 beats row18 when applied
# later, and still beats a subsequent row20. PPL getters above remain LOCAL.
local_height = ink_height('row_after_column', 'A1', red=True)
assert ink_height('row_after_both_columns', 'A1', red=True) == local_height
assert ink_height('column_after_row', 'B1') + 3 < ink_height('row_after_column', 'B1')
assert ink_height('row_after_both_columns', 'B1') == ink_height('column_after_row', 'B1')
assert ink_height('row_after_both_columns', 'B1') + 3 < ink_height('row_after_both_columns', 'C1')
assert ink_height('font_auto', 'A1', red=True) > local_height + 3

# Typed2x2 matrices are actually stacked only for effective Textbook Yes.
assert ink_height('cell_textbook_modes', 'A2', red=True) > 25
assert ink_height('cell_textbook_modes', 'B2') < 20
assert ink_height('cell_textbook_modes', 'C2') < 20
assert ink_height('sheet_textbook_yes', 'A2', red=True) > 25
assert ink_height('sheet_textbook_yes', 'B2') < 20
assert ink_height('sheet_textbook_yes', 'C2') > 25
assert all(ink_height('rational_decimal_modes', name) < 20 for name in ['A5', 'B5', 'C5'])

# Full32bit colors remain distinct. The inherited Auto palette entry must not
# consume the legitimate raw0x01000000 value; UIREAD independently checks it.
def fill_color(label, address):
    image = cell_pixels(label, address)
    colors = image.getcolors(image.width * image.height)
    return max(colors, key=lambda item: item[0])[1]

opaque, partial, faint = [fill_color('sheet_textbook_yes', name) for name in ['A3', 'B3', 'C3']]
assert opaque[2] > 230 and max(opaque[:2]) < 20, opaque
assert 20 < partial[0] < faint[0] < 255 and partial[2] > 230 and faint[2] > 230, (partial, faint)
assert min(fill_color('fill_auto', 'A3')) > 240
assert ink_height('text_color_auto', 'A4', red=True) > 5

final_sheet = sheet(states['final'])
for state in restored.values():
    assert sheet(state) == final_sheet
    assert {'SHEETUI', 'UIREAD'} <= set(state['calculator_programs'])
for address in ['A2', 'B2', 'C2']:
    value = restored['reopened_grid']['native_model_results']['Spreadsheet'][address]['Object']['typed']
    assert value['type'] == 'matrix' and (value['rows'], value['cols']) == (2, 2), value
assert ink_height('reopened_grid', 'A2', red=True) > 25
assert ink_height('reopened_grid', 'B2') < 20
assert ink_height('reopened_grid', 'C2') > 25
print('Spreadsheet attributes: real UI scope precedence in both orders, local PPL getters/formula recalculation, Auto restoration, typed matrix Textbook, decimal fractions, fullARGB palette and restart passed')
