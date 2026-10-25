"""Run with: python3 -m unittest discover -s scripts/tests -p 'test_*.py'."""

import binascii
import contextlib
import io
import json
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

from scripts.research.ck_inspect import PE
from scripts.research.ck_screen_replay import (
    assemble,
    inspect,
    inspect_logs,
    replay,
)


def png(width=1, height=1):
    def chunk(kind, data):
        return (
            struct.pack(">I", len(data))
            + kind
            + data
            + struct.pack(">I", binascii.crc32(kind + data))
        )

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(
            b"IDAT", zlib.compress((b"\0" + b"\x12\x34\x56" * width) * height)
        )
        + chunk(b"IEND", b"")
    )


def response(fmt=9, x=65535, y=65535, width=1, height=1):
    body = bytes([fmt]) + struct.pack("<HH", x, y) + png(width, height)
    return (
        b"\xfc\x01"
        + struct.pack(">IH", len(body) + 2, binascii.crc_hqx(body, 0))
        + body
    )


def log(message, fmt=9):
    rows = [f"0.000s host→calc 4 B 00 00 FC {fmt:02X}"]
    for i, off in enumerate(range(0, len(message), 63)):
        report = bytes([i % 255]) + message[off : off + 63]
        report = report.ljust(64, b"\xa5")
        rows.append(f"0.001s calc→host 64 B {report.hex(' ')}")
    return "\n".join(rows)


class ScreenEvidenceTests(unittest.TestCase):
    def test_ordered_reports_discard_padding_and_validate_crc(self):
        message = response()
        fmt, actual, reports = assemble(log(message))
        self.assertEqual(actual, message)
        self.assertEqual(fmt, 9)
        self.assertEqual(reports, 2)
        metadata, _ = inspect(fmt, actual)
        self.assertTrue(metadata["full_origin_sentinel"])
        self.assertEqual(metadata["png_dimensions"], [1, 1])

    def test_crc_sequence_truncation_and_extent_failures(self):
        message = response()
        corrupted = bytearray(message)
        corrupted[-1] ^= 1
        with self.assertRaisesRegex(ValueError, "CRC"):
            inspect(9, corrupted)
        with self.assertRaisesRegex(ValueError, "sequence"):
            assemble(log(message).replace("64 B 01 ", "64 B 02 "))
        with self.assertRaisesRegex(ValueError, "incomplete"):
            assemble(log(message).rsplit("\n", 1)[0])
        with self.assertRaisesRegex(ValueError, "outside"):
            inspect(9, response(x=320, y=0))
        with self.assertRaisesRegex(ValueError, "exactly one"):
            assemble(log(message) + "\n0.002s host→calc 4 B 00 00 FC 09")

    def test_sequence_wrap_and_completion_are_independent_of_padding(self):
        # Opaque assembly fixture extends beyond one 255-report sequence cycle.
        message = b"\xfc\x01" + struct.pack(">I", 17000) + bytes(17000)
        _, actual, reports = assemble(log(message))
        self.assertEqual(actual, message)
        self.assertGreater(reports, 255)

    def test_patch_replay_requires_a_baseline_and_preserves_other_pixels(self):
        with tempfile.TemporaryDirectory() as tmp:
            full = Path(tmp) / "full.log"
            patch = Path(tmp) / "patch.log"
            full.write_text(log(response(width=320, height=240)))
            patch.write_text(log(response(x=12, y=14)))
            rows = list(replay([full, patch]))
            self.assertEqual(rows[0]["changed_pixels"], 320 * 240)
            self.assertEqual(rows[1]["changed_pixels"], 0)
            with self.assertRaisesRegex(ValueError, "without a full baseline"):
                list(replay([patch]))


class PEInspectionTests(unittest.TestCase):
    def test_hand_built_pe_mapping_disassembly_xrefs_and_pointers(self):
        # Independent tiny PE32 fixture: call 401010; ret; padding; ret.
        data = bytearray(0x400)
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 0x3C, 0x80)
        data[0x80:0x84] = b"PE\0\0"
        struct.pack_into("<HH", data, 0x84, 0x14C, 1)
        struct.pack_into("<H", data, 0x94, 0xE0)
        struct.pack_into("<H", data, 0x98, 0x10B)
        struct.pack_into("<I", data, 0xB4, 0x400000)
        data[0x178:0x180] = b".text\0\0\0"
        struct.pack_into("<IIII", data, 0x180, 0x100, 0x1000, 0x200, 0x200)
        struct.pack_into("<I", data, 0x19C, 0x60000020)
        data[0x200:0x206] = bytes.fromhex("e8 0b 00 00 00 c3")
        data[0x210] = 0xC3
        struct.pack_into("<I", data, 0x220, 0x401010)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "fixture.exe"
            path.write_bytes(data)
            pe = PE(path)
            self.assertEqual(pe.offset(0x401010), 0x210)
            self.assertEqual(pe.va(0x220), 0x401020)
            self.assertEqual(pe.imports, {})
            with self.assertRaises(ValueError):
                pe.offset(0x500000)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                pe.disasm(0x401000, 0x401006)
                pe.xrefs([0x401010])
                pe.pointers(0x401010)
            self.assertIn("00401000 call     0x401010", out.getvalue())
            self.assertIn("00401020 file=0x220", out.getvalue())


class AdditionalPEInspectionTests(unittest.TestCase):
    def test_pe32_and_pe64_named_and_ordinal_imports(self):
        for bits in (32, 64):
            with (
                self.subTest(bits=bits),
                tempfile.TemporaryDirectory() as temporary,
            ):
                data = bytearray(0x600)
                data[:2] = b"MZ"
                struct.pack_into("<I", data, 0x3C, 0x80)
                data[0x80:0x84] = b"PE\0\0"
                optional_size = 0xE0 if bits == 32 else 0xF0
                base = 0x400000 if bits == 32 else 0x140000000
                struct.pack_into(
                    "<HH", data, 0x84, 0x14C if bits == 32 else 0x8664, 1
                )
                struct.pack_into("<H", data, 0x94, optional_size)
                struct.pack_into(
                    "<H", data, 0x98, 0x10B if bits == 32 else 0x20B
                )
                struct.pack_into(
                    "<I" if bits == 32 else "<Q",
                    data,
                    0x98 + (28 if bits == 32 else 24),
                    base,
                )
                directories = 0x98 + (96 if bits == 32 else 112)
                struct.pack_into("<I", data, directories + 8, 0x1100)
                section = 0x98 + optional_size
                data[section : section + 8] = b".text\0\0\0"
                struct.pack_into(
                    "<IIII", data, section + 8, 0x400, 0x1000, 0x400, 0x200
                )
                struct.pack_into("<I", data, section + 36, 0x60000020)
                struct.pack_into(
                    "<IIIII", data, 0x300, 0x1140, 0, 0, 0x1180, 0x1160
                )
                word_format = "<I" if bits == 32 else "<Q"
                struct.pack_into(word_format, data, 0x340, 0x1190)
                struct.pack_into(
                    word_format,
                    data,
                    0x340 + bits // 8,
                    (1 << (bits - 1)) | 17,
                )
                data[0x380:0x389] = b"demo.dll\0"
                data[0x392:0x39B] = b"ReadFile\0"
                path = Path(temporary) / "synthetic.exe"
                path.write_bytes(data)
                pe = PE(path)
                self.assertEqual(pe.bits, bits)
                self.assertEqual(
                    pe.imports,
                    {
                        base + 0x1160: "demo.dll!ReadFile",
                        base + 0x1160 + bits // 8: "demo.dll!ordinal:17",
                    },
                )

    def test_truncated_pe_is_an_actionable_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "truncated.exe"
            path.write_bytes(b"MZ")
            with self.assertRaisesRegex(ValueError, "truncated PE data"):
                PE(path)


class PreservedEvidenceTests(unittest.TestCase):
    """Optional private inputs stay in place and are never rewritten."""

    def test_all_100_saved_logs_and_0d_replay_match_recorded_reports(self):
        root = Path(__file__).resolve().parents[2]
        logs = sorted(
            (
                root
                / "artifacts/live-mirror/planning-baseline-20261024T213653Z"
            ).glob("format-*.hid.log")
        )
        reports = (
            root
            / "scratch/research/autonomous-control-investigation"
            / "continuation-20261025"
        )
        metadata = reports / "preserved-format-metadata.json"
        replay_report = reports / "preserved-0d-replay.json"
        if not logs or not metadata.exists() or not replay_report.exists():
            self.skipTest("private preserved logs/reports are unavailable")
        self.assertEqual(len(logs), 100)
        self.assertEqual(inspect_logs(logs), json.loads(metadata.read_text()))
        patches = [path for path in logs if path.name.startswith("format-0d-")]
        self.assertEqual(
            list(replay(patches)), json.loads(replay_report.read_text())
        )

    def test_pinned_static_trace_reproduces_recorded_file_hashes(self):
        from scripts.research.ck_trace_evidence import write_evidence

        root = Path(__file__).resolve().parents[2]
        research = root / "scratch/research/autonomous-control-investigation"
        binary = research / "ck/ConnectivityKit.exe"
        manifest = research / "continuation-20261025/trace-v1/manifest.json"
        if not binary.exists() or not manifest.exists():
            self.skipTest(
                "private Connectivity Kit binary/manifest are unavailable"
            )
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "trace"
            with contextlib.redirect_stdout(io.StringIO()):
                write_evidence(PE(binary), output)
            actual = json.loads((output / "manifest.json").read_text())
            expected = json.loads(manifest.read_text())
            self.assertEqual(actual["sha256"], expected["sha256"])
            self.assertEqual(actual["files"], expected["files"])


if __name__ == "__main__":
    unittest.main()
