#!/usr/bin/env python3
"""Capture self-authored retained-firmware controls in disposable profiles.

This is an evidence collector, not a parity verdict. Failed imports remain
failures; later probes must not be mistaken for successful source binding.
"""
import argparse
import hashlib
import json
from pathlib import Path
import queue
import time

from benchmark_run import Session, copy_initial_profile


def controls():
    probe = 'EXPORT M19RESET() BEGIN A:=0; RETURN A; END;\nEXPORT M19PROBE() BEGIN RETURN A; END;\n'
    failed = 'M19MAKE() BEGIN A:=A+1; RETURN 1/0; END;\nLOCAL count:=M19MAKE();\nEXPORT M19READ() BEGIN RETURN count; END;\n'
    return [
        ('initializer_retry', [('store', 'ZZT_M19PROBE', probe), ('run', 'ZZT_M19PROBE', 'M19RESET'),
          ('store', 'ZZT_M19FAIL', failed), ('run', 'ZZT_M19PROBE', 'M19PROBE'),
          ('run', 'ZZT_M19FAIL', 'M19READ'), ('run', 'ZZT_M19PROBE', 'M19PROBE'),
          ('run', 'ZZT_M19FAIL', 'M19READ'), ('run', 'ZZT_M19PROBE', 'M19PROBE')]),
        ('duplicate_exports', [
          ('store', 'ZZT_M19ONE', 'EXPORT M19GLOBAL:=11;\nEXPORT M19ONE() BEGIN RETURN M19GLOBAL; END;\n'),
          ('run', 'ZZT_M19ONE', 'M19ONE'),
          ('store', 'ZZT_M19TWO', 'EXPORT M19GLOBAL:=22;\nEXPORT M19TWO() BEGIN RETURN M19GLOBAL; END;\n'),
          ('run', 'ZZT_M19ONE', 'M19ONE'), ('run', 'ZZT_M19TWO', 'M19TWO'),
          ('store', 'ZZT_M19USE', 'EXPORT M19USE() BEGIN RETURN M19GLOBAL; END;\n'),
          ('run', 'ZZT_M19USE', 'M19USE')]),
        ('typed_returns', [('store', 'ZZT_M19TYPES', '''EXPORT M19RAT() BEGIN RETURN CAS("1/3"); END;
EXPORT M19SYM() BEGIN RETURN CAS("x+1"); END;
EXPORT M19NEST() BEGIN RETURN {{1+2*i,3-4*i},{5}}; END;
EXPORT M19INT() BEGIN RETURN #FFFFFFFFFFFFFFFF:64h; END;
'''), *[('run', 'ZZT_M19TYPES', entry) for entry in ['M19RAT', 'M19SYM', 'M19NEST', 'M19INT']]])]


def collect(session, command, timeout=90):
    session.send(command)
    records = []
    deadline = time.monotonic() + timeout
    while True:
        event = session.events.get(timeout=max(0, deadline-time.monotonic()))
        if isinstance(event, Exception):
            raise event
        records.append(event)
        if event.get('event') == 'result' and event.get('request') == command['request']:
            return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--helper', type=Path, required=True)
    parser.add_argument('--assets', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--case', action='append', choices=['initializer_retry','duplicate_exports','duplicate_exports_reverse','typed_returns'])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    report = {'scope': 'firmware_control_capture', 'physical_hardware_verified': False,
              'helper_sha256': hashlib.sha256(args.helper.read_bytes()).hexdigest(),
              'firmware_sha256': hashlib.sha256((args.assets/'armfir.elf').read_bytes()).hexdigest(),
              'cases': []}
    try:
        selected = controls()
        duplicate = selected[1][1]
        selected.append(('duplicate_exports_reverse', [duplicate[i] for i in [2,4,0,1,4,5,6]]))
        for name, steps in selected:
            if args.case and name not in args.case:
                continue
            directory = args.output/name
            directory.mkdir()
            copy_initial_profile(args.assets/'initial-data', directory/'profile')
            session = Session(args.helper.resolve(), args.assets.resolve(), directory/'profile', directory)
            case = {'name': name, 'requests': [], 'status': 'capturing'}
            report['cases'].append(case)
            identities = {}
            try:
                session.wait(lambda event: event.get('state') == 'ready', 90)
                for request, (action, program, value) in enumerate(steps, 1):
                    command = {'request': request, 'name': program}
                    if action == 'store':
                        identities[program] = hashlib.sha256(value.encode()).hexdigest()
                        command.update(command='store_program', source=value, expected_previous_sha256=None)
                    else:
                        command.update(command='run_stored', source_sha256=identities[program], entry=value, arguments='')
                    events = collect(session, command)
                    case['requests'].append({'command': command, 'events': events})
                    (args.output/'capture.json').write_text(json.dumps(report, indent=2)+'\n')
                session.shutdown()
                case['status'] = 'captured'
            except (OSError, RuntimeError, ValueError, EOFError, queue.Empty) as error:
                case.update(status='capture_failed', error=str(error))
            finally:
                session.close()
    finally:
        (args.output/'capture.json').write_text(json.dumps(report, indent=2)+'\n')
    return int(any(case['status'] != 'captured' for case in report['cases']))


if __name__ == '__main__':
    raise SystemExit(main())
