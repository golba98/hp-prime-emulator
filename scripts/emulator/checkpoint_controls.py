#!/usr/bin/env python3
"""Run isolated real-firmware OFF controls with neutral and latched Shift.

No USB/device access. Copies user-supplied initial storage; never opens a live
calculator profile. Default expects both checkpoints to save. For an unfixed
helper, --expected-latched failed verifies the historical defect explicitly.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import time
import zlib

from benchmark_run import Session

SHIFT = [0, 1800, 2204, 190, 1820, 2076, 2204, 1820]
UPPER = [0, 253980, 131106, 65570, 32830, 16418, 8226, 256674]
LOWER = [0, 0, 0, 253980, 65568, 32828, 16418, 256700]
LOCKED_UPPER = [0, 255752, 133276, 67774, 36764, 18588, 10396, 256156]


def profile_identity(directory):
    digest = hashlib.sha256()
    for path in sorted(directory.rglob('*')):
        if path.is_symlink():
            raise ValueError(f'initial profile contains a symlink: {path}')
        name = path.relative_to(directory).as_posix().encode()
        digest.update(struct.pack('<I', len(name)))
        digest.update(name)
        if path.is_dir():
            digest.update(b'D')
        elif path.is_file():
            digest.update(b'F')
            digest.update(struct.pack('<Q', path.stat().st_size))
            with path.open('rb') as source:
                for chunk in iter(lambda: source.read(1024 * 1024), b''):
                    digest.update(chunk)
        else:
            raise ValueError(f'initial profile contains a nonregular entry: {path}')
    return digest.hexdigest()


def copy_initial_profile(source, destination):
    expected = profile_identity(source)
    shutil.copytree(source, destination, symlinks=True)
    if profile_identity(destination) != expected:
        raise ValueError('initial profile changed while copying')
    return expected


def status_indicators(frame):
    """Exact local-firmware glyph masks; unknown patterns stay explicit."""
    if frame is None or len(frame) != 16 + 320 * 240 * 4:
        return {'shift': None, 'alpha': 'unobserved'}
    pixels = struct.unpack('<76800I', frame[16:])
    shift, alpha = [], []
    recognized = True
    for y in range(8):
        shift_row, alpha_row = 0, 0
        for x in range(34):
            pixel = pixels[y * 320 + x]
            red, green, blue = (pixel >> 16) & 255, (pixel >> 8) & 255, pixel & 255
            if red < 60 and green > 180 and blue > 180 and x < 14:
                shift_row |= 1 << x
            elif red > 150 and green > 120 and blue < 100 and x >= 14:
                alpha_row |= 1 << (x - 14)
            elif not (red < 60 and 65 <= green <= 120 and 140 <= blue <= 190):
                recognized = False
        shift.append(shift_row)
        alpha.append(alpha_row)
    alpha_name = ('upper' if alpha == UPPER else 'lower' if alpha == LOWER else
                  'locked_upper' if alpha == LOCKED_UPPER else 'off' if alpha == [0] * 8 else 'unknown')
    return {'shift': (True if shift == SHIFT else False if shift == [0] * 8 else None) if recognized else None,
            'alpha': alpha_name if recognized else 'unknown', 'shift_mask': shift, 'alpha_mask': alpha}


def shift_indicator(frame):
    return status_indicators(frame)['shift']


def save_frame(frame, destination):
    """Keep exact protocol pixels and a lossless PNG for visual inspection."""
    destination.with_suffix('.frame').write_bytes(frame)
    pixels = struct.unpack('<76800I', frame[16:])
    raw = b''.join(b'\0' + bytes(component for pixel in pixels[y * 320:(y + 1) * 320]
                                for component in [(pixel >> 16) & 255, (pixel >> 8) & 255, pixel & 255])
                   for y in range(240))

    def chunk(kind, body):
        return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body) & 0xffffffff)

    destination.with_suffix('.png').write_bytes(
        b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 320, 240, 8, 2, 0, 0, 0))
        + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))


class RecordingSession(Session):
    def __init__(self, helper, assets, profile, output):
        self.started = time.monotonic()
        self.records = []
        self.latest = None
        self.inspection_id = 100
        super().__init__(helper, assets, profile, output)

    def read(self):
        try:
            while True:
                size, kind = struct.unpack('<IB', self.read_exact(5))
                if size > 4 * 1024 * 1024:
                    raise ValueError('oversized session packet')
                data = self.read_exact(size)
                if kind == 1:
                    event = json.loads(data)
                    self.records.append({'elapsed_s': time.monotonic() - self.started, 'event': event})
                    self.events.put(event)
                elif kind == 2:
                    self.latest = data
        except (EOFError, OSError, ValueError) as error:
            self.events.put(error)

    def send(self, command):
        self.records.append({'elapsed_s': time.monotonic() - self.started, 'command': command})
        super().send(command)

    def inspect(self):
        self.inspection_id += 1
        request = self.inspection_id
        self.send({'command': 'inspect', 'request': request})
        return self.wait(lambda e: e.get('event') == 'inspection' and e['request'] == request, 10)['report']

    def idle(self, after=-1):
        deadline = time.monotonic() + 15
        while True:
            report = self.inspect()
            state = report['resources']
            pressed = state.get('typing_last_keydown_main_wait')
            acknowledged = (after < 0 or (pressed is not None and pressed >= after
                                          and state['main_wait_generation'] > pressed))
            if (acknowledged and state.get('main_ui_idle') and state['main_wait_generation'] > after
                    and not state['input_pending'] and not state['touch_pending']
                    and not state['typing_pending'] and not state['guest_call_busy']):
                return report
            if time.monotonic() > deadline:
                raise TimeoutError('firmware did not acknowledge a fresh main-UI idle boundary')
            time.sleep(.05)

    def capture(self, path):
        before = struct.unpack_from('<Q', self.latest)[0] if self.latest else -1
        deadline = time.monotonic() + 5
        while True:
            # A forced capture can be refused if the memory gate is busy. Retry
            # the request; never treat an unchanged LCD as a missing frame.
            self.send({'command': 'frame_request'})
            if self.latest is not None and struct.unpack_from('<Q', self.latest)[0] > before:
                break
            if time.monotonic() >= deadline:
                raise TimeoutError('no acknowledged forced LCD capture')
            time.sleep(.05)
        frame = self.latest
        save_frame(frame, path)
        return {'generation': struct.unpack_from('<Q', frame)[0], **status_indicators(frame)}


def control(session, output, mode, expected, wake_after=False, probe_shift=False):
    latched = mode == 'shift'
    result = {'mode': mode, 'prelatched_shift': latched, 'expected_checkpoint': expected}
    failure = None
    try:
        session.wait(lambda e: e.get('event') == 'lifecycle' and e.get('state') == 'ready', 60)
        initial = session.idle()
        result['initial_inspection'] = initial
        result['initial_frame'] = session.capture(output / 'initial')
        if result['initial_frame']['shift'] is not False:
            raise AssertionError('initial storage must boot into a verified neutral Shift state')
        keys = {'neutral': [], 'shift': [0x8b], 'alpha': [0xb6], 'alpha_lock': [0xb6, 0xb6]}[mode]
        for code in keys:
            baseline = initial['resources']['main_wait_generation']
            session.send({'command': 'key', 'code': code, 'down': True})
            session.send({'command': 'key', 'code': code, 'down': False})
            initial = session.idle(baseline)
        if keys:
            result['latched_inspection'] = initial
            result['latched_frame'] = session.capture(output / 'latched')
            if latched and result['latched_frame']['shift'] is not True:
                raise AssertionError('physical Shift was not observed latched before checkpoint')
            if mode.startswith('alpha') and result['latched_frame']['alpha'] in ('off', 'unknown', 'unobserved'):
                raise AssertionError('physical Alpha was not observed before checkpoint')
        if probe_shift and mode.startswith('alpha'):
            for phase in ['alpha_with_shift', 'alpha_restored']:
                baseline = initial['resources']['main_wait_generation']
                session.send({'command': 'key', 'code': 0x8b, 'down': True})
                session.send({'command': 'key', 'code': 0x8b, 'down': False})
                initial = session.idle(baseline)
                result[phase + '_inspection'] = initial
                result[phase + '_frame'] = session.capture(output / phase)
            restored = result['alpha_restored_frame']
            if restored['shift'] is not False or restored['alpha'] != result['latched_frame']['alpha']:
                raise AssertionError('Shift probe did not restore the verified initial Alpha state')
        started = time.monotonic()
        session.send({'command': 'checkpoint', 'id': 11, 'wake': False})
        result['checkpoint'] = session.wait(lambda e: e.get('event') == 'checkpoint' and e.get('id') == 11, 40)
        result['checkpoint_seconds'] = time.monotonic() - started
        if result['checkpoint']['result']['status'] != 'saved':
            result['failure_inspection'] = session.inspect()
            result['failure_frame'] = session.capture(output / 'after_failure')
        if result['checkpoint']['result']['status'] != expected:
            raise AssertionError(f"unexpected checkpoint: {result['checkpoint']}")
        if wake_after and result['checkpoint']['result']['status'] == 'saved':
            session.send({'command': 'key', 'code': 0x83, 'down': True})
            session.send({'command': 'key', 'code': 0x83, 'down': False})
            session.wait(lambda e: e.get('event') == 'lifecycle' and e.get('state') == 'ready', 30)
            result['wake_inspection'] = session.idle(initial['resources']['main_wait_generation'])
            result['wake_frame'] = session.capture(output / 'after_wake')

    except Exception as error:
        failure = str(error)
    finally:
        try:
            # Cleanup is independently verified even after a failed assertion.
            session.shutdown()
            statuses = [r['event'].get('message') for r in session.records
                        if r.get('event', {}).get('event') == 'status']
            if 'The calculator saved its state and switched off' not in statuses:
                raise AssertionError('normal PowerDown did not report authoritative firmware OFF')
            if (output / 'profile/.recovery/unclean').exists():
                raise AssertionError('normal PowerDown left an unclean recovery marker')
            result['normal_power_down'] = 'saved_and_closed'
        except Exception as error:
            result['cleanup_error'] = str(error)
            failure = failure or str(error)
    result['passed'] = failure is None
    if failure is not None:
        result['error'] = failure
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--helper', required=True, type=Path)
    parser.add_argument('--assets', type=Path, default=Path.home() / '.local/share/hp-prime-emulator/firmware')
    parser.add_argument('--initial-data', type=Path)
    parser.add_argument('--output', required=True, type=Path, help='new evidence directory; never overwritten')
    parser.add_argument('--expected-latched', choices=['saved', 'failed'], default='saved')
    parser.add_argument('--modes', default='neutral,shift', help='comma-separated neutral,shift,alpha,alpha_lock')
    parser.add_argument('--wake-after', action='store_true', help='after a saved checkpoint, observe a real ON wake before cleanup')
    parser.add_argument('--probe-shift', action='store_true', help='observe and undo one physical Shift tap in each Alpha mode before checkpoint')
    args = parser.parse_args()
    modes = args.modes.split(',')
    if not modes or len(modes) != len(set(modes)) or any(mode not in ['neutral', 'shift', 'alpha', 'alpha_lock'] for mode in modes):
        parser.error('--modes requires distinct supported modes')
    helper, assets = args.helper.resolve(strict=True), args.assets.resolve(strict=True)
    initial = (args.initial_data or assets / 'initial-data').resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=False)
    report = {'version': 1, 'origin': 'local-firmware', 'physical_verification': False,
              'helper_sha256': hashlib.sha256(helper.read_bytes()).hexdigest(),
              'firmware_sha256': hashlib.sha256((assets / 'armfir.elf').read_bytes()).hexdigest(), 'cases': []}
    for mode in modes:
        output = args.output / mode
        output.mkdir()
        identity = copy_initial_profile(initial, output / 'profile')
        session = RecordingSession(helper, assets, output / 'profile', output)
        try:
            result = control(session, output, mode, args.expected_latched if mode == 'shift' else 'saved', args.wake_after, args.probe_shift)
        finally:
            session.close()
            (output / 'events.json').write_text(json.dumps(session.records, indent=2) + '\n')
        result['initial_profile_sha256'] = identity
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        report['cases'].append(result)
        print(json.dumps({k: result[k] for k in ['mode', 'passed', 'checkpoint_seconds', 'normal_power_down', 'latched_frame', 'wake_frame', 'error'] if k in result}), flush=True)
    report['passed'] = all(case['passed'] for case in report['cases'])
    (args.output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
