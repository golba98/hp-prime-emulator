import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import full_system_build as builder
import full_system_run as runner


class ExecutionTests(unittest.TestCase):
    def good(self):
        return {'initial_pc':0x30000000, 'initial_mailbox':0,
                'stepped_pc':0x30000004, 'stopped_pc':0x3000005c,
                'halt_address':0x3000005c, 'mailbox':0x600d0001,
                'aliases':[runner.PATTERN]*4}

    def test_observations_require_actual_entry_step_halt_and_all_aliases(self):
        self.assertEqual(runner.validate_observation(self.good()), self.good())
        for field in ['initial_pc','initial_mailbox','stepped_pc','stopped_pc','halt_address','mailbox']:
            bad=self.good();bad[field]=123
            with self.subTest(field=field), self.assertRaises(ValueError):
                runner.validate_observation(bad)
        bad=self.good();bad['aliases'][-1]^=1
        with self.assertRaises(ValueError):runner.validate_observation(bad)

    def test_generic_arm_result_does_not_prove_hp_prime_aliases(self):
        generic={'initial_pc':0x10000,'stepped_pc':0x10004,'mailbox':0x600d0001}
        with self.assertRaises(ValueError):runner.validate_observation(generic)

    def test_elf_identity_mismatch_is_rejected_before_output_or_container(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);binary=root/'build/arm-softmmu/qemu-system-arm'
            binary.parent.mkdir(parents=True);binary.write_bytes(b'old executable')
            meta={'schema_version':1,'status':'built','reference_revision':runner.PIN,'binary_sha256':runner.digest(binary)}
            (root/'build.json').write_text(json.dumps(meta))
            self.assertEqual(runner.checked_manifest(root),meta)
            binary.write_bytes(b'changed executable')
            with self.assertRaises(ValueError):runner.run(root,root/'out')
            self.assertFalse((root/'out').exists())

    @patch.object(builder,'git',return_value='wrong reference')
    def test_wrong_source_pin_never_creates_build(self, _):
        with tempfile.TemporaryDirectory() as folder:
            out=Path(folder)/'build'
            with self.assertRaises(ValueError):builder.build(Path(folder),out)
            self.assertFalse(out.exists())

    @patch.object(builder,'git',return_value=runner.PIN)
    def test_existing_build_directory_is_never_overwritten(self, _):
        with tempfile.TemporaryDirectory() as folder:
            out=Path(folder);sentinel=out/'keep';sentinel.write_bytes(b'state')
            with self.assertRaises(FileExistsError):builder.build(out,out)
            self.assertEqual(sentinel.read_bytes(),b'state')


if __name__=='__main__':unittest.main()
