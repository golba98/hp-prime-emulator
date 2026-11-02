"""Replay captured observations and enforce NAND experiment evidence boundaries."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

from full_system_nand import main_ecc, image_bytes, run, validate_observation


class NandEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = Path(__file__).resolve().parents[2]/'research/full-system/evidence/m19-nand.json'
        cls.runs = json.loads(path.read_text())['runs']

    def test_all_actual_captures_replay(self):
        for record in self.runs:
            self.assertEqual(record['status'], 'passed')
            self.assertTrue(record['guest']['linked'])
            self.assertTrue(record['guest_cpu_execution_verified'])
            self.assertFalse(record['physical_hardware_verified'])
            self.assertFalse(record['kernel_boot_verified'])
            self.assertEqual(record['flash']['base_sha256'],record['flash']['base_after_sha256'])
            validate_observation(copy.deepcopy(record['observations']),record['case'],
                                 bytes.fromhex(record['observations']['uart_hex']),record['guest'])

    def test_reject_forged_success_fields(self):
        for record in self.runs:
            changes = [('initial_pc',123),('stepped_pc',123),('initial_mailbox',1),
                       ('stopped_pc',123),('halt_address',0),('cpsr',0x1f)]
            for key,value in changes:
                with self.subTest(case=record['case'],field=key):
                    obs=copy.deepcopy(record['observations']);obs[key]=value
                    with self.assertRaises(ValueError):
                        validate_observation(obs,record['case'],bytes.fromhex(record['observations']['uart_hex']),record['guest'])

    def test_reject_wrong_uart(self):
        for record in self.runs:
            with self.assertRaises(ValueError):
                validate_observation(record['observations'],record['case'],b'host fabricated success',record['guest'])

    def test_each_claimed_mailbox_field_required(self):
        for record in self.runs:
            indices={'nand':[0,2,3,4,5,6,8,9,10,11,12,13],
                     'nand_ecc':[0,2,3,4,5,6,7],'boot_chain':[0,2,3,4,5]}[record['case']]
            for index in indices:
                with self.subTest(case=record['case'],word=index):
                    obs=copy.deepcopy(record['observations']);obs['mailbox'][index]^=1
                    with self.assertRaises(ValueError):
                        validate_observation(obs,record['case'],bytes.fromhex(record['observations']['uart_hex']),record['guest'])

    def test_bounded_mailbox_types(self):
        r=self.runs[0]
        for value in [-1,0x100000000,True,'1611464729']:
            obs=copy.deepcopy(r['observations']);obs['mailbox'][0]=value
            with self.assertRaises(ValueError):validate_observation(obs,r['case'],b'M19 NAND\n',r['guest'])

    def test_ecc_fixed_asymmetric_reference(self):
        data=bytes((((i*37+11)&255)^(4 if i==201 else 0))for i in range(512))
        self.assertEqual(main_ecc(data),0x5f99a596)
        self.assertEqual(sum(data),65284)
        self.assertEqual(main_ecc(b'\xff'*512),0xffffffff)
        self.assertEqual(main_ecc(b'\x00'*512),0xffffffff)
        for value in [b'',b'\0'*2049]:
            with self.assertRaises(ValueError):main_ecc(value)

    def test_reject_bad_elf(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'bad.elf';path.write_bytes(b'not ARM elf')
            with self.assertRaises(ValueError):image_bytes(path)

    def test_invalid_request_never_creates_output(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)/'output'
            for case,deadline in [('bad',30),('nand',0),('nand',121)]:
                with self.assertRaises(ValueError):run(Path('/nonexistent'),output,case,deadline)
            self.assertFalse(output.exists())


if __name__ == '__main__':unittest.main()
