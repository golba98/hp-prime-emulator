import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('preflight', Path(__file__).with_name('full_system_preflight.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class InventoryTests(unittest.TestCase):
    @patch.object(module.shutil, 'which', return_value=None)
    def test_missing_tools_cannot_imply_kernel_boot(self, _):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = module.inventory(root / 'absent', [root / 'missing'])
            self.assertFalse(report['kernel_boot_verified'])
            self.assertFalse(report['artifact_bootability_verified'])
            self.assertEqual(report['artifacts'][0]['status'], 'missing')
            self.assertEqual(len(report['blockers']), 4)

    def test_smoke_build_refuses_existing_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sentinel = root / 'keep'
            sentinel.write_bytes(b'preserve')
            with self.assertRaises(FileExistsError):
                module.build_smoke(root, {})
            self.assertEqual(sentinel.read_bytes(), b'preserve')

    def test_inventory_is_hashed_and_read_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = root / 'asset'
            artifact.write_bytes(b'abc')
            report = module.inventory(root / 'absent', [artifact])
            self.assertEqual(report['artifacts'][0]['sha256'], 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')
            self.assertEqual(artifact.read_bytes(), b'abc')
            self.assertFalse(report['artifact_bootability_verified'])


if __name__ == '__main__':
    unittest.main()
