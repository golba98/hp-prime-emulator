"""Offline validation contracts; these fixtures are not guest-execution evidence."""
import copy
from pathlib import Path
import tempfile
import unittest
from full_system_clock import build_guest, validate_observation


class ClockValidationTests(unittest.TestCase):
    def fixture(self):
        words = [0] * 64
        words[0:12] = [0x600d0019, 96, 48, 0, 0xfeed0019, 0x19abcdef,
                       0xc, 0, 0, 0x20, 0xc, 0]
        return {'initial_pc': 0x30000000, 'initial_mailbox': 0,
                'stepped_pc': 0x30000004, 'stopped_pc': 0x30000300,
                'halt_address': 0x30000300, 'mailbox': words}

    def test_validates_independent_clock_ratios_and_reset(self):
        observed = validate_observation(self.fixture(), 'clock_reset', b'CSRYZ')
        self.assertEqual(observed['virtual_time']['expected_loop_ns'], 96000)

    def test_rejects_missing_execution_or_clock_effect(self):
        original = self.fixture()
        mutations = [('initial_pc', 0), ('initial_mailbox', 1), ('stepped_pc', 0x30000000),
                     ('stopped_pc', 0), ('halt_address', 0)]
        for field, value in mutations:
            data = copy.deepcopy(original)
            data[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                validate_observation(data, 'clock_reset', b'CSRYZ')
        for index, value in [(1, 48), (2, 96), (3, 1), (5, 0), (6, 4), (8, 1), (10, 0)]:
            data = copy.deepcopy(original)
            data['mailbox'][index] = value
            with self.subTest(index=index), self.assertRaises(ValueError):
                validate_observation(data, 'clock_reset', b'CSRYZ')

    def test_rejects_unbounded_or_noninteger_mailbox(self):
        for value in [-1, 2**32, True, '96']:
            data = self.fixture()
            data['mailbox'][1] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                validate_observation(data, 'clock_reset', b'CSRYZ')

    def test_rejects_extra_gated_uart_character(self):
        with self.assertRaises(ValueError):
            validate_observation(self.fixture(), 'clock_reset', b'CSRYXZ')

    def test_builds_self_authored_arm_guest_and_preserves_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'guest'
            result = build_guest(output, 'clock_reset')
            self.assertTrue(result['linked'])
            self.assertFalse(result['executed'])
            before = (output / 'hardware.elf').read_bytes()
            with self.assertRaises(FileExistsError):
                build_guest(output, 'clock_reset')
            self.assertEqual(before, (output / 'hardware.elf').read_bytes())


if __name__ == '__main__':
    unittest.main()
