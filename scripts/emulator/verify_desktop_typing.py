#!/usr/bin/env python3
"""Compare real firmware desktop paste with equivalent physical key sequences.

Uses only an isolated copy of initialized local firmware storage. It never
opens USB, changes the user's profile or claims physical-device verification.
Every invocation retains its input frames and exact diff counts as evidence.
"""
import argparse
import json
import pathlib
import queue
import shutil
import struct
import subprocess
import threading
import time
import zlib


def png(path, pixels):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    rgb = bytearray()
    for y in range(240):
        rgb.append(0)
        for x in range(320):
            value = struct.unpack_from('<I', pixels, 4 * (320 * y + x))[0]
            rgb.extend(((value >> 16) & 255, (value >> 8) & 255, value & 255))
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 320, 240, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rgb)) + chunk(b'IEND', b''))


class Session:
    def __init__(self, args):
        self.args = args
        self.events = queue.Queue()
        self.frame = None
        self.request = 1
        self.error = None
        self.log = (args.out / 'stderr.log').open('wb')
        command = [str(args.runner), 'session', '--firmware', str(args.firmware), '--data-dir', str(args.out / 'profile')]
        if args.os_rom:
            command += ['--os-rom', str(args.os_rom)]
        self.child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log)
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def read(self):
        def exact(n):
            result = bytearray()
            while len(result) < n:
                data = self.child.stdout.read(n - len(result))
                if not data:
                    raise EOFError('firmware helper closed its output')
                result.extend(data)
            return result
        try:
            while True:
                header = exact(5)
                size, kind = struct.unpack('<IB', header)
                if size > 4 * 1024 * 1024:
                    raise ValueError('oversized firmware packet')
                data = exact(size)
                if kind == 2:
                    if size != 16 + 320 * 240 * 4:
                        raise ValueError('bad framebuffer length')
                    self.frame = bytes(data[16:])
                elif kind == 1:
                    event = json.loads(data)
                    if event['event'] == 'error':
                        self.error = event['message']
                    self.events.put(event)
        except (EOFError, ValueError, OSError) as error:
            self.events.put(dict(event='reader_error', message=str(error)))

    def send(self, command, **fields):
        payload = json.dumps(dict(command=command, **fields)).encode()
        self.child.stdin.write(struct.pack('<IB', len(payload), 3) + payload)
        self.child.stdin.flush()

    def wait(self, predicate, timeout=90):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            event = self.events.get(timeout=max(.01, deadline - time.monotonic()))
            if event['event'] in ('error', 'reader_error'):
                raise RuntimeError(event['message'])
            if predicate(event):
                return event
        raise TimeoutError('firmware event timeout')

    def settled(self):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            request = self.request
            self.request += 1
            self.send('inspect', request=request)
            event = self.wait(lambda event: event.get('event') == 'inspection' and event['request'] == request, 10)
            resources = event['report']['resources']
            if resources['typing_pending'] == 0 and resources['queued_keys'] == 0 and resources['input_pending'] == 0 and resources['pump_parked'] and resources['main_ui_idle']:
                self.send('frame_request')
                time.sleep(.3)
                if self.error:
                    raise RuntimeError(self.error)
                if self.frame is None:
                    raise RuntimeError('no firmware framebuffer')
                return self.frame
            time.sleep(.05)
        raise TimeoutError('typing did not reach an actual idle firmware boundary')

    def keys(self, codes):
        for code in codes:
            self.send('key', code=code, down=True)
            self.send('key', code=code, down=False)

    def close(self):
        if self.child.poll() is None:
            try:
                self.send('shutdown')
                self.child.wait(timeout=10)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                self.child.kill()
                self.child.wait()
        self.log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['runner', 'firmware', 'init-data', 'out']:
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    parser.add_argument('--os-rom', type=pathlib.Path)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    shutil.copytree(args.init_data, args.out / 'profile')
    session = Session(args)
    report = []
    try:
        session.wait(lambda event: event['event'] == 'lifecycle' and event['state'] == 'ready', 180)
        for index, (expression, keys) in enumerate([
            ('2^3+4', [0x5a, 0x46, 0x33, 0x04, 0xb9, 0x55]),
            ('2^(3+4)^5+6', [0x5a, 0x46, 0x4e, 0x33, 0xb9, 0x55, 0x04, 0x46, 0x56, 0x04, 0x04, 0xb9, 0x57]),
            ('π×2−ⅈ÷3', [0x8b, 0x33, 0x58, 0x5a, 0xb7, 0x8b, 0x5a, 0x54, 0x33]),
        ]):
            session.keys([0x0001])  # Esc clears the editable expression.
            session.settled()
            session.send('text', text=expression, paste=True)
            pasted = session.settled()
            png(args.out / f'{index}-paste.png', pasted)
            session.keys([0x0001])
            session.settled()
            session.keys(keys)
            control = session.settled()
            png(args.out / f'{index}-keys.png', control)
            # Firmware clock and modifier annunciators occupy rows 0–19.
            # The complete expression/history/menu area must match exactly.
            different = sum(pasted[i:i+4] != control[i:i+4] for i in range(20*320*4, len(pasted), 4))
            report.append(dict(expression=expression, different_pixels=different, exact=different == 0, evidence='local_firmware'))
            if different:
                raise AssertionError(f'{expression}: {different} pixels differ between paste and actual key input')
    finally:
        session.close()
        (args.out / 'report.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
