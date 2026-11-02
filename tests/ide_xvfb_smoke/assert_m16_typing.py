#!/usr/bin/env python3
"""Desktop typing checked against actual firmware key sequences, not hardware.

Run run_m12.sh m16_typing 180, then pass its scenario evidence directory.
Screenshots must come from this run; no physical-calculator claim is made.
"""
import json
import sys
from pathlib import Path
from assert_m12 import Report, png, region

root = Path(sys.argv[1])
r = Report()
dump = json.loads((root / 'dump.json').read_text())
r.check('all GUI waits completed', not dump['wait_timeouts'])
r.check('firmware remained alive', dump['firmware_alive'])
for name, crop in [('home', (0, 14, 200, 218)),
                   ('cas', (0, 14, 200, 218)),
                   ('editor', (8, 23, 19, 37))]:
    typed, control = png(root / f'{name}_typed.png'), png(root / f'{name}_control.png')
    r.check(f'{name} aA matches firmware key control',
            region(typed, *crop) == region(control, *crop))
r.check('latched Alpha preserves desktop aA',
        region(png(root / 'editor_alpha_lock.png'), 8, 23, 19, 37) ==
        region(png(root / 'editor_control.png'), 8, 23, 19, 37))
r.check('pasted expression preserves paired-parenthesis semantics',
        region(png(root / 'paste_expression.png'), 0, 50, 200, 218) ==
        region(png(root / 'expression_control.png'), 0, 50, 200, 218))
r.check('brackets match explicit firmware template and Right',
        region(png(root / 'editor_brackets.png'), 8, 47, 19, 37) ==
        region(png(root / 'brackets_control.png'), 8, 47, 19, 37))
for suffix in ['multiline', 'power']:
    # Ignore the clock and blinking input cursor, compare all history pixels.
    r.check(f'unsafe {suffix} paste neither types nor executes',
            region(png(root / f'rejected_{suffix}.png'), 0, 320, 20, 200) ==
            region(png(root / 'paste_result.png'), 0, 320, 20, 200) and
            region(png(root / f'rejected_{suffix}.png'), 5, 320, 202, 219) ==
            region(png(root / 'paste_result.png'), 5, 320, 202, 219))
sys.exit(0 if all(x['pass'] for x in r.results) else 1)
