#!/usr/bin/env python3
"""Inspect a disposable firmware session, including boot failures. Never opens USB."""
import argparse
import json
from pathlib import Path
import queue
import shutil
import struct
import subprocess
import tempfile
import threading
import time


def exact(stream, count):
    result = bytearray()
    while len(result) < count:
        block = stream.read(count - len(result))
        if not block:
            raise EOFError('helper closed its output')
        result.extend(block)
    return result


def events(stream, output):
    try:
        while True:
            size, kind = struct.unpack('<IB', exact(stream, 5))
            if size > 4 * 1024 * 1024:
                raise ValueError('oversized helper packet')
            data = exact(stream, size)
            if kind == 1:
                output.put(json.loads(data))
    except (EOFError, OSError, ValueError) as error:
        output.put(error)


def send(child, command):
    payload = json.dumps(command).encode()
    child.stdin.write(struct.pack('<IB', len(payload), 3) + payload)
    child.stdin.flush()


def inspect(args):
    if args.boot_timeout <= 0 or args.reply_timeout <= 0:
        raise ValueError('timeouts must be positive')
    initial = args.initial_data.resolve()
    if initial.is_symlink() or any(p.is_symlink() for p in initial.rglob('*')):
        raise ValueError('initialized test storage must not contain symlinks')
    with tempfile.TemporaryDirectory(prefix='prime-inspection-') as directory:
        profile = Path(directory) / 'profile'
        for drive in ['A', 'C']:
            shutil.copytree(initial / drive, profile / drive)
        command = [str(args.helper.resolve()), 'session', '--firmware', str(args.firmware.resolve()), '--data-dir', str(profile)]
        if args.os_rom:
            command += ['--os-rom', str(args.os_rom.resolve())]
        with (Path(directory) / 'stderr').open('w') as errors:
            child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors)
            output = queue.Queue(maxsize=64)
            reader = threading.Thread(target=events, args=(child.stdout, output), daemon=True)
            reader.start()
            try:
                ready, hello = False, False
                deadline = time.monotonic() + args.boot_timeout
                while time.monotonic() < deadline:
                    try:
                        event = output.get(timeout=max(0.001, deadline-time.monotonic()))
                    except queue.Empty:
                        break
                    if isinstance(event, Exception):
                        raise event
                    if event.get('event') == 'hello':
                        if event.get('version') != 7:
                            raise ValueError('inspection requires helper protocol7')
                        hello = True
                    if event.get('event') == 'lifecycle' and event.get('state') == 'ready':
                        ready = True
                        break
                if not hello:
                    raise ValueError('no verified protocol handshake')
                send(child, {'command': 'inspect', 'request': 1})
                deadline = time.monotonic() + args.reply_timeout
                while True:
                    remaining = deadline-time.monotonic()
                    if remaining <= 0:
                        raise TimeoutError('inspection reply deadline expired')
                    event = output.get(timeout=remaining)
                    if isinstance(event, Exception):
                        raise event
                    if event.get('event') in ['error', 'closed']:
                        raise ValueError(event.get('message'))
                    if event.get('event') == 'inspection':
                        if event.get('request') != 1 or event['report']['version'] != 1:
                            raise ValueError('inspection identity/version mismatch')
                        report = event['report']
                        if report['readiness']['live_restore_ready']:
                            raise ValueError('unsupported live-restore capability')
                        return {'inspection_collected': True, 'ready_observed_before_query': ready,
                                'session_ready': report['lifecycle'] == 'ready', 'report': report}
            finally:
                try:
                    send(child, {'command': 'shutdown'})
                    child.wait(timeout=5)
                except (OSError, subprocess.TimeoutExpired):
                    child.kill()
                    child.wait()
                # A bounded queue may leave the daemon reader awaiting a consumer;
                # its process lifetime ends with this one-shot tool.


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['helper', 'firmware', 'initial-data']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--os-rom', type=Path)
    parser.add_argument('--boot-timeout', type=float, default=30)
    parser.add_argument('--reply-timeout', type=float, default=5)
    args = parser.parse_args()
    try:
        print(json.dumps(inspect(args), indent=2))
        return 0
    except (OSError, ValueError, EOFError, queue.Empty) as error:
        print(json.dumps({'inspection_collected': False, 'error': str(error)}))
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
