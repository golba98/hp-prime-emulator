"""Oracle/failure regressions; actual ARM executions are separately retained."""
import tempfile
from pathlib import Path
import unittest
import full_system_display as display


class DisplayControls(unittest.TestCase):
    def test_capture_oracle_checks_every_byte_and_dimensions(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)/'frame.ppm'
            pixels = display.expected_frame()
            path.write_bytes(b'P6\n320 240\n255\n'+pixels)
            self.assertEqual(display.validate_ppm(path), pixels)
            changed = bytearray(pixels)
            changed[(173*320+91)*3+2] ^= 1
            path.write_bytes(b'P6\n320 240\n255\n'+changed)
            with self.assertRaisesRegex(ValueError, 'differs in 1 RGB bytes'):
                display.validate_ppm(path)
            path.write_bytes(b'P6\n240 320\n255\n'+pixels)
            with self.assertRaisesRegex(ValueError, 'exact 320x240'):
                display.validate_ppm(path)
            path.write_bytes(b'P6\n320 240\n255\n'+pixels[:-1])
            with self.assertRaises(ValueError):
                display.validate_ppm(path)

    def test_pageflip_and_updated_pixel_are_independent_of_capture(self):
        first, second = display.expected_frame(), display.expected_frame(True)
        offset = (23*320+17)*3
        self.assertEqual(second[offset:offset+3], b'\xff\0\xff')
        self.assertNotEqual(first, second)
        self.assertEqual(first[:3], b'\0\0\0')
        self.assertEqual(second[:3], b'\xff\xff\xff')

    def test_touch_observation_rejects_stale_status_and_coordinates(self):
        sample = {'touch_hex': '090e0510'+'ff'*16}
        display.validate_touch(sample, 9, 225, 80)
        with self.assertRaisesRegex(ValueError, 'status/coordinates'):
            display.validate_touch(sample, 10, 225, 80)
        with self.assertRaises(ValueError):
            display.validate_touch(sample, 9, 224, 80)

    def test_invalid_case_does_not_create_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)/'output'
            with self.assertRaises(ValueError):
                display.run(Path(tmp), path, 'kernel')
            self.assertFalse(path.exists())

    def test_guest_build_refuses_existing_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)
            marker = path/'preserve'
            marker.write_bytes(b'original')
            with self.assertRaises(FileExistsError):
                display.build_guest(path)
            self.assertEqual(marker.read_bytes(), b'original')


if __name__ == '__main__':
    unittest.main()
