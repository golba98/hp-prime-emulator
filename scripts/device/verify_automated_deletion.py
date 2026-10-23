#!/usr/bin/env python3
"""Opt-in physical acceptance test; deletes only fresh objects this run creates.

Uses the ordinary Rust CLI and its ownership/backup guards. No key injection,
helper execution, firmware operation or guessed USB packet is sent.
"""
import argparse
import hashlib
import json
from pathlib import Path
import secrets
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--hardware', action='store_true', required=True,
                        help='Explicitly enable physical object creation/deletion.')
    parser.add_argument('--ppl', default='target/debug/ppl')
    parser.add_argument('--output-dir', default='artifacts/automatic-deletion-tests')
    args = parser.parse_args()
    suffix = secrets.token_hex(3).upper()
    program, note, sentinel = [f'ZZT_{suffix}{kind}' for kind in ('P', 'N', 'S')]
    root = Path(args.output_dir) / suffix
    root.mkdir(parents=True, exist_ok=False)
    source = root / 'test.hpprgm'
    source.write_text(f'EXPORT {program}()\nBEGIN\n  RETURN 42;\nEND;\n')
    note_source = root / 'note.txt'
    note_source.write_text(f'Disposable deletion acceptance fixture {suffix}.\n')
    report = {'status': 'UNRESOLVED', 'program': program, 'note': note,
              'sentinel': sentinel, 'steps': [], 'execution_claimed': False}

    def record():
        (root / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')

    def run(label, command, typed_name=None):
        step = {'label': label, 'command': ['ppl', 'device', *command],
                'state': 'submitting'}
        report['steps'].append(step)
        record()
        result = subprocess.run([args.ppl, 'device', *command], text=True,
                                input=(typed_name + '\n') if typed_name else '',
                                capture_output=True, check=False, timeout=600)
        (root / f'{label}.stdout').write_text(result.stdout)
        (root / f'{label}.stderr').write_text(result.stderr)
        step.update(state='returned', exit_code=result.returncode)
        record()
        if result.returncode:
            raise RuntimeError(f'{label} failed; stopped without retrying. See {root}')
        print(f'{label}: passed', flush=True)
        return result

    def delete_and_verify(label, kind, name):
        result = run(f'delete-{label}', ['delete', kind, name, '--json'])
        value = json.loads(result.stdout)
        if not (value['deletion_confirmed'] is True
                and value['mechanism'] == 'usb_delete'
                and value['helper'] is None
                and value['execution_confirmed'] is False
                and value['error'] is None):
            raise RuntimeError(f'Unexpected deletion result for {name}; evidence retained')
        backup_dir = Path(value['backup_directory'])
        backup = backup_dir / f'{kind}-{name}.bin'
        if hashlib.sha256(backup.read_bytes()).hexdigest() != value['results'][0]['backup_sha256']:
            raise RuntimeError('Backup does not match recorded SHA-256')
        # A separate CLI process opens a new HID connection for this query.
        check = run(f'verify-{label}', ['delete', 'verify', kind, name, '--json'])
        if json.loads(check.stdout)['deletion_confirmed'] is not True:
            raise RuntimeError(f'Independent absence query failed for {name}')

    try:
        run('send-program', ['send', program, str(source)], program)
        run('send-note', ['send', note, str(note_source), '--note'], note)
        run('send-sentinel', ['send', sentinel, str(note_source), '--note'], sentinel)
        before, after = root / 'sentinel-before.bin', root / 'sentinel-after.bin'
        run('read-sentinel-before', ['get', 'note', sentinel, '--out', str(before)])
        delete_and_verify('program', 'program', program)
        delete_and_verify('note', 'note', note)
        run('read-sentinel-after', ['get', 'note', sentinel, '--out', str(after)])
        if before.read_bytes() != after.read_bytes():
            raise RuntimeError('Sentinel changed; retain all evidence for investigation')
        report['sentinel_unchanged'] = True
        delete_and_verify('sentinel', 'note', sentinel)
        report['status'] = 'VERIFIED'
        record()
        print(f'Unattended program/note deletion verified. Evidence: {root}')
        return 0
    except (RuntimeError, ValueError, OSError, subprocess.SubprocessError) as error:
        report['error'] = str(error)
        record()
        print(f'{error}\nNo automatic retry or speculative cleanup was attempted.', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
