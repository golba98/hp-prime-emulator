"""Normal CI validates actual recorded synthetic observations without QEMU/USB."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import full_system_hardware as hardware
import full_system_preflight as preflight


class HardwareEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = Path(__file__).resolve().parents[2] / 'research/full-system/evidence/m18-hardware.json'
        cls.evidence = json.loads(path.read_text())

    def test_real_recorded_mailboxes_validate_without_promoting_provenance(self):
        self.assertEqual(len(self.evidence['repeated_execution']), 12)
        for case, report in self.evidence['cases'].items():
            with self.subTest(case=case):
                obs = copy.deepcopy(report['observations'])
                self.assertEqual(hardware.validate_observation(obs, case, bytes.fromhex(obs['uart_hex'])), obs)
                self.assertTrue(report['guest_cpu_execution_verified'])
                self.assertFalse(report['kernel_boot_verified'])
                self.assertFalse(report['physical_hardware_verified'])

    def test_false_irq_counts_masking_priority_mode_and_returns_are_rejected(self):
        original = self.evidence['cases']['timer_irq']['observations']
        for word in [0, 1, 4, 5, 6, 9, 11, 12, 13, 17, 25]:
            with self.subTest(word=word), self.assertRaises(ValueError):
                bad = copy.deepcopy(original)
                bad['mailbox'][word] = 0
                hardware.validate_observation(bad, 'timer_irq', bytes.fromhex(original['uart_hex']))

    def test_fiq_requires_actual_handler_mode_and_no_irq_execution(self):
        original = self.evidence['cases']['timer_fiq']['observations']
        for word, value in [(1, 1), (2, 0), (10, 0x12)]:
            with self.subTest(word=word), self.assertRaises(ValueError):
                bad = copy.deepcopy(original)
                bad['mailbox'][word] = value
                hardware.validate_observation(bad, 'timer_fiq', b'FFFE')

    def test_fault_types_addresses_cache_state_and_unmapped_fetch_are_semantic(self):
        original = self.evidence['cases']['mmu']['observations']
        for word in [3, 7, 8, 11, 12, 14, 15, 34, 35, 36, 37, 38, 39]:
            with self.subTest(word=word), self.assertRaises(ValueError):
                bad = copy.deepcopy(original)
                bad['mailbox'][word] ^= 1
                hardware.validate_observation(bad, 'mmu', b'')

    def test_uart_bytes_and_gpio_readbacks_are_exact(self):
        original = self.evidence['cases']['uart_gpio']['observations']
        with self.assertRaises(ValueError):
            hardware.validate_observation(copy.deepcopy(original), 'uart_gpio', b'M18 UART\r\n')
        with self.assertRaises(ValueError):
            bad = copy.deepcopy(original)
            bad['mailbox'][8] ^= 1
            hardware.validate_observation(bad, 'uart_gpio', b'M18 UART\n')

    def test_mailbox_and_execution_identity_reject_malformed_observations(self):
        original = self.evidence['cases']['timer_irq']['observations']
        for field in ('initial_pc', 'initial_mailbox', 'stepped_pc', 'stopped_pc', 'halt_address'):
            with self.subTest(field=field), self.assertRaises(ValueError):
                bad = copy.deepcopy(original)
                bad[field] ^= 4
                hardware.validate_observation(bad, 'timer_irq', bytes.fromhex(original['uart_hex']))
        for value in (True, -1, 0x100000000, '10'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                bad = copy.deepcopy(original)
                bad['mailbox'][17] = value
                hardware.validate_observation(bad, 'timer_irq', bytes.fromhex(original['uart_hex']))

    def test_unknown_observation_case_cannot_infer_success(self):
        original = self.evidence['cases']['mmu']['observations']
        with self.assertRaises(ValueError):
            hardware.validate_observation(copy.deepcopy(original), 'unverified', b'')

    def test_invalid_case_deadline_and_existing_outputs_fail_before_mutation(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            out = root/'new'
            for case, limit in [('absent', 30), ('mmu', 0), ('mmu', 121)]:
                with self.subTest(case=case, limit=limit), self.assertRaises(ValueError):
                    hardware.run(root, out, case, limit)
                self.assertFalse(out.exists())
            sentinel = root/'keep'
            sentinel.write_bytes(b'original')
            with self.assertRaises(FileExistsError):
                hardware.build_guest(root, 'mmu')
            self.assertEqual(sentinel.read_bytes(), b'original')


class IsolatedRuntimeTests(unittest.TestCase):
    def runtime(self, root):
        binary = root/'build/arm-softmmu/qemu-system-arm'
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b'controlled executable')
        data = {'schema_version':1, 'status':'built', 'reference_revision':hardware.PIN,
                'image':'owned-test-image', 'image_id':'test-image-id',
                'binary_sha256':hardware.digest(binary)}
        (root/'build.json').write_text(json.dumps(data))
        return binary

    def tools(self, name):
        if name in ('python2', 'qemu-system-arm', 'rustc'):
            return None
        return '/tools/'+name

    def test_validated_cached_runtime_does_not_need_host_qemu_python2_or_source(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.runtime(root)
            with patch.object(preflight.shutil, 'which', side_effect=self.tools), \
                    patch.object(preflight.subprocess, 'check_output', return_value='test-image-id\n'):
                report = preflight.inventory(root/'no-source', [], root)
            self.assertEqual(report['blockers'], [])
            self.assertEqual(report['isolated_runtime']['status'], 'validated')
            self.assertFalse(report['kernel_boot_verified'])

    def test_changed_binary_or_runtime_image_cannot_validate(self):
        for change in ('binary', 'image'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                binary = self.runtime(root)
                if change == 'binary':
                    binary.write_bytes(b'changed')
                with patch.object(preflight.shutil, 'which', side_effect=self.tools), \
                        patch.object(preflight.subprocess, 'check_output', return_value='wrong-image\n'):
                    report = preflight.inventory(root/'no-source', [], root)
                self.assertIn('isolated_runtime_invalid', report['blockers'])
                self.assertEqual(report['isolated_runtime']['status'], 'invalid')


if __name__ == '__main__':
    unittest.main()
