"""Synthetic fixtures for archive boundaries and MSVC/Qt x64 layouts."""

import contextlib
import ctypes.util
import io
import struct
import tempfile
import unittest
from pathlib import Path

from scripts.research.ck_inspect import PE
from scripts.research.official_archives import cabinets, check_listing


class ArchiveTests(unittest.TestCase):
    def test_cabinet_bounds_reject_incomplete_downloads(self):
        header = b"MSCF" + bytes(4) + struct.pack("<I", 40) + bytes(28)
        self.assertEqual(list(cabinets(b"prefix" + header)), [(6, header)])
        with self.assertRaisesRegex(ValueError, "beyond"):
            list(cabinets(header[:-1]))

    def test_archive_listing_refuses_traversal_absolute_links_and_oversize(
        self,
    ):
        for entry in [
            "Path = ../outside\nSize = 1",
            "Path = /outside\nSize = 1",
            "Path = C:\\outside\nSize = 1",
            "Path = x\nSize = 1\nSymbolic Link = y",
            "Path = x\nSize = 536870913",
        ]:
            with self.subTest(entry=entry), self.assertRaises(ValueError):
                check_listing("----------\n" + entry)
        check_listing("----------\nPath = images/icon.png\nSize = 20\n")


class X64InspectionTests(unittest.TestCase):
    def fixture(self, directory):
        base = 0x140000000
        data = bytearray(0x1200)
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 0x3C, 0x80)
        data[0x80:0x84] = b"PE\0\0"
        struct.pack_into("<HH", data, 0x84, 0x8664, 1)
        struct.pack_into("<H", data, 0x94, 0xF0)
        struct.pack_into("<H", data, 0x98, 0x20B)
        struct.pack_into("<Q", data, 0xB0, base)
        data[0x188:0x190] = b".rdata\0\0"
        struct.pack_into("<IIII", data, 0x190, 0x1000, 0x1000, 0x1000, 0x200)
        struct.pack_into("<I", data, 0x1AC, 0x60000020)

        def put(rva, content):
            offset = rva - 0x1000 + 0x200
            data[offset : offset + len(content)] = content

        put(0x1110, b".?AVPanel@@\0")
        put(0x1200, struct.pack("<6I", 1, 16, 0, 0x1100, 0x1300, 0x1200))
        put(0x1248, struct.pack("<QQ", base + 0x1200, base + 0x1400))
        # LEA RCX,[RIP+0x1f9] resolves from 1400+7 to 1600.
        put(0x1400, bytes.fromhex("48 8d 0d f9 01 00 00 c3"))
        put(
            0x1500,
            struct.pack(
                "<6Q", 0, base + 0x1600, base + 0x1700, base + 0x1400, 0, 0
            ),
        )
        put(0x1600, struct.pack("<iiIIq", -1, 5, 0, 0, 24) + b"Panel\0")
        put(
            0x1700,
            struct.pack("<14I", 8, 0, 0, 0, 0, 14, 0, 0, 0, 0, 0, 0, 0, 0),
        )
        path = Path(directory) / "synthetic.exe"
        path.write_bytes(data)
        return PE(path)

    def test_rtti_native_pointers_relative_locator_and_qt_24_byte_headers(
        self,
    ):
        with tempfile.TemporaryDirectory() as directory:
            pe = self.fixture(directory)
            self.assertEqual(
                list(pe.vtables(0x140001100)), [(0x140001200, 16, 0x140001250)]
            )
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                pe.rtti("Panel")
                pe.qt(0x140001500)
                pe.disasm(0x140001400, 0x140001408)
                pe.xrefs([0x140001600])
            actual = output.getvalue()
            self.assertIn("vtable=0x140001250", actual)
            self.assertIn("Qt Panel", actual)
            self.assertIn("RIP target 0x140001600", actual)
            self.assertEqual(actual.count("140001400 lea"), 2)

    def test_invalid_self_relative_locator_is_not_a_vtable(self):
        with tempfile.TemporaryDirectory() as directory:
            pe = self.fixture(directory)
            data = bytearray(pe.data)
            struct.pack_into("<I", data, pe.offset(0x140001214), 0x1201)
            pe.path.write_bytes(data)
            self.assertEqual(list(PE(pe.path).vtables(0x140001100)), [])


class QtPixelOracleTests(unittest.TestCase):
    @unittest.skipUnless(
        ctypes.util.find_library("Qt5Gui"), "system Qt5Gui unavailable"
    )
    def test_system_qt_expands_all_native_words_including_unused_high_bit(
        self,
    ):
        from scripts.research.qt555_oracle import convert

        raw = b"".join(word.to_bytes(2, "little") for word in range(65536))
        actual = convert(raw, 256, 256)
        expected = bytearray()
        for word in range(65536):
            for shift in (10, 5, 0):
                five = (word >> shift) & 31
                expected.append(five * 8 + five // 4)
        self.assertEqual(actual, expected)


if __name__ == "__main__":
    unittest.main()
