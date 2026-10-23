from pathlib import Path

from src.ppl_emulator.source_loader import read_ppl_file


def test_read_ppl_file_strips_utf16_wrapper_and_trailing_garbage(tmp_path: Path):
    payload = (
        'EXPORT T()\r\n'
        'BEGIN\r\n'
        'PRINT("OK");\r\n'
        'END;\r\n'
    ).encode('utf-16-le')
    raw = b'\x01\x02junk' + payload + b'\x80\x00\xff\xfftail'
    path = tmp_path / 'wrapped.hpprgm'
    path.write_bytes(raw)

    text = read_ppl_file(str(path))

    assert text == 'EXPORT T()\nBEGIN\nPRINT("OK");\nEND;'
