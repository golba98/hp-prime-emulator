import unittest
from check_pixel_geometry import check, expected


class PixelGeometryTests(unittest.TestCase):
    def test_detects_one_pixel_shift_and_unexpected_ink(self):
        image = [[expected(x, y) for x in range(320)] for y in range(240)]
        self.assertTrue(check(image)["passed"])
        image[100][100], image[100][101] = image[100][101], image[100][100]
        self.assertEqual(check(image)["mismatches"], 2)
        image[0][0] = (0, 0, 0)
        self.assertEqual(check(image)["mismatches"], 3)

    def test_refuses_resized_capture(self):
        with self.assertRaises(ValueError):
            check([[(255, 255, 255)]])


if __name__ == "__main__":
    unittest.main()
