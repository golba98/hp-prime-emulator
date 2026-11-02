#!/usr/bin/env python3
"""Offline controls for the capture harness. Mock outcomes are not HLE evidence."""
import json
import struct
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from checkpoint_controls import RecordingSession, SHIFT, UPPER, LOCKED_UPPER, control, copy_initial_profile, shift_indicator, status_indicators


def frame(shift=False, generation=1, alpha=None):
    pixels = [0x065696] * (320 * 240)
    if shift:
        for y, row in enumerate(SHIFT):
            for x in range(14):
                if row & (1 << x):
                    pixels[y * 320 + x] = 0x00f0f0
    if alpha:
        for y,row in enumerate(alpha):
            for x in range(20):
                if row & (1 << x):
                    pixels[y * 320 + 14 + x] = 0xffff00
    return struct.pack('<QQ', generation, 0) + struct.pack('<76800I', *pixels)


class MockSession:
    def __init__(self, outcome='saved', unknown=False):
        self.outcome = outcome
        self.unknown = unknown
        self.records = []
        self.commands = []
        self.cleaned = False

    def wait(self, predicate, timeout):
        if not any(c['command'] == 'checkpoint' for c in self.commands):
            return {'event': 'lifecycle', 'state': 'ready'}
        return {'event': 'checkpoint', 'id': 11, 'result': {'status': self.outcome}}

    def idle(self, after=-1):
        return {'resources': {'main_wait_generation': after + 2}}

    def inspect(self):
        return {'resources': {}}

    def capture(self, path):
        alpha_count = sum(c.get('code') == 0xb6 and c.get('down') for c in self.commands)
        return {'generation': 1, 'shift': None if self.unknown else path.name == 'latched' and not alpha_count,
                'alpha': 'locked_upper' if alpha_count == 2 else 'upper' if alpha_count else 'off'}

    def send(self, command):
        self.commands.append(command)

    def shutdown(self):
        self.cleaned = True
        self.records.append({'event': {'event': 'status', 'message': 'The calculator saved its state and switched off'}})


class CheckpointControlTests(unittest.TestCase):
    def test_exact_shift_and_unknown_status_bars(self):
        self.assertIs(shift_indicator(frame()), False)
        self.assertIs(shift_indicator(frame(True)), True)
        self.assertIsNone(shift_indicator(None))
        self.assertIsNone(shift_indicator(b'bad'))
        unknown = struct.pack('<QQ', 1, 0) + struct.pack('<76800I', *([0xffffff] * 76800))
        self.assertIsNone(shift_indicator(unknown))

    def test_alpha_and_shift_glyphs_remain_distinct(self):
        self.assertEqual(status_indicators(frame(alpha=UPPER))['alpha'], 'upper')
        self.assertEqual(status_indicators(frame(alpha=LOCKED_UPPER))['alpha'], 'locked_upper')
        combined = status_indicators(frame(shift=True, alpha=LOCKED_UPPER))
        self.assertTrue(combined['shift'])
        self.assertEqual(combined['alpha'], 'locked_upper')

    def test_alpha_controls_observe_wake_and_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            for mode, count in [('alpha', 1), ('alpha_lock', 2)]:
                session = MockSession()
                result = control(session, Path(directory), mode, 'saved', wake_after=True)
                self.assertTrue(result['passed'], result)
                self.assertTrue(session.cleaned)
                self.assertIn('wake_frame', result)
                keys = [c['code'] for c in session.commands if c['command'] == 'key' and c['down']]
                self.assertEqual(keys, [0xb6] * count + [0x83])

    def test_actual_firmware_modifier_captures(self):
        fixture = json.loads((Path(__file__).parent / 'fixtures/m17_modifier_controls.json').read_text())
        self.assertEqual(fixture['origin'], 'local-firmware')
        self.assertIs(fixture['physical_verification'], False)
        for case in fixture['cases']:
            pixels = [0x065696] * (320 * 240)
            for y, row in enumerate(case['pixels']):
                pixels[y * 320:y * 320 + 34] = row
            observed = status_indicators(struct.pack('<QQ', 1, 0) + struct.pack('<76800I', *pixels))
            self.assertEqual(observed['shift'], case['expected']['shift'], case['id'])
            self.assertEqual(observed['alpha'], case['expected']['alpha'], case['id'])

    def test_control_checks_neutral_latched_and_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            for mode, expected in [('neutral', 'saved'), ('shift', 'saved'), ('shift', 'failed')]:
                session = MockSession(expected)
                result = control(session, Path(directory), mode, expected)
                self.assertTrue(result['passed'], result)
                self.assertTrue(session.cleaned)
                keys = [c for c in session.commands if c['command'] == 'key']
                self.assertEqual(keys, [{'command': 'key', 'code': 0x8b, 'down': True},
                                        {'command': 'key', 'code': 0x8b, 'down': False}] if mode == 'shift' else [])

    def test_unexpected_checkpoint_failure_cannot_skip_cleanup_or_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            session = MockSession('failed')
            result = control(session, Path(directory), 'shift', 'saved')
            self.assertFalse(result['passed'])
            self.assertTrue(session.cleaned)
            self.assertIn('unexpected checkpoint', result['error'])

    def test_unknown_initial_status_does_not_send_keys_and_still_cleans_up(self):
        with tempfile.TemporaryDirectory() as directory:
            session = MockSession(unknown=True)
            result = control(session, Path(directory), 'shift', 'saved')
            self.assertFalse(result['passed'])
            self.assertTrue(session.cleaned)
            self.assertEqual(session.commands, [])

    def test_idle_requires_actual_press_ack_not_an_unrelated_timer_wait(self):
        session = RecordingSession.__new__(RecordingSession)
        def report(generation, press):
            return {'resources': {'main_ui_idle': True, 'main_wait_generation': generation,
                                  'typing_last_keydown_main_wait': press, 'input_pending': 0,
                                  'touch_pending': 0, 'typing_pending': 0, 'guest_call_busy': False}}
        reports = [report(11, None), report(12, 9), report(13, 10)]
        seen = []
        def inspect():
            seen.append(True)
            return reports.pop(0)
        session.inspect = inspect
        with patch('checkpoint_controls.time.sleep'):
            result = session.idle(10)
        self.assertEqual(len(seen), 3)
        self.assertEqual(result['resources']['typing_last_keydown_main_wait'], 10)

    def test_forced_capture_retries_a_busy_gate_and_preserves_pixels(self):
        with tempfile.TemporaryDirectory() as directory:
            session = RecordingSession.__new__(RecordingSession)
            session.latest = frame(generation=1)
            calls = []

            def request(command):
                calls.append(command)
                if len(calls) == 2:
                    session.latest = frame(True, generation=2)

            session.send = request
            target = Path(directory) / 'observed'
            with patch('checkpoint_controls.time.sleep'):
                result = session.capture(target)
            self.assertEqual(len(calls), 2)
            self.assertTrue(result['shift'])
            self.assertEqual(target.with_suffix('.frame').read_bytes(), frame(True, generation=2))
            self.assertTrue(target.with_suffix('.png').read_bytes().startswith(b'\x89PNG\r\n\x1a\n'))

    def test_initial_profile_copy_refuses_links_and_preserves_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            source.mkdir()
            (source / 'file').write_bytes(b'calculator data')
            identity = copy_initial_profile(source, root / 'copy')
            self.assertEqual(len(identity), 64)
            self.assertEqual((source / 'file').read_bytes(), b'calculator data')
            (source / 'link').symlink_to(source / 'file')
            with self.assertRaisesRegex(ValueError, 'symlink'):
                copy_initial_profile(source, root / 'refused')
            self.assertFalse((root / 'refused').exists())


if __name__ == '__main__':
    unittest.main()
