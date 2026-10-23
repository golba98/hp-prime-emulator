import os
import re

_UTF16_MARKERS = (
    '#pragma',
    'EXPORT ',
    'export ',
    'PROCEDURE ',
    'procedure ',
    '// ',
)

_ALLOWED_UNICODE = frozenset(
    '≠≤≥▶→←∞≡−π√∑∫∂θℯⅈ°²³'
    'ΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩ'
    'αβγδεζηθικλμνξοπρστυφχψω'
)


def _looks_like_utf16_hp_file(sample: bytes) -> bool:
    return sum(1 for b in sample if b == 0) > 10


def _find_utf16_payload_start(raw: bytes) -> int:
    starts: list[int] = []
    for marker in _UTF16_MARKERS:
        idx = raw.find(marker.encode('utf-16-le'))
        if idx != -1:
            starts.append(idx)
    if not starts:
        return 0
    start = min(starts)
    return start if start % 2 == 0 else start - 1


def _is_allowed_char(ch: str) -> bool:
    if ch in '\t\n\r':
        return True
    if ch in _ALLOWED_UNICODE:
        return True
    code = ord(ch)
    if 0x20 <= code <= 0x7E:
        return True
    if 0xA0 <= code <= 0xFF:
        return True
    return False


def _sanitize_text(text: str) -> str:
    text = text.replace('\ufeff', '')
    text = text.replace('\x00', '')
    text = text.replace('\ufffd', '')
    text = ''.join(ch if _is_allowed_char(ch) else ' ' for ch in text)
    text = text.replace('\r\n', '\n').replace('\r', '\n')
    text = '\n'.join(line.rstrip() for line in text.split('\n'))
    text = re.sub(r'\n{3,}', '\n\n', text)
    return text.strip()


def _trim_to_program_bounds(text: str) -> str:
    start = -1
    for marker in _UTF16_MARKERS:
        idx = text.find(marker)
        if idx != -1 and (start == -1 or idx < start):
            start = idx
    if start > 0:
        text = text[start:]

    lines = text.splitlines()
    while lines and not lines[0].strip():
        lines.pop(0)

    last_end_idx = -1
    last_end_line = ''
    for i, line in enumerate(lines):
        match = re.search(r'\bEND;?', line, re.IGNORECASE)
        if match:
            last_end_idx = i
            suffix = line[match.end():].strip()
            if not suffix or suffix.startswith('//'):
                last_end_line = line
            else:
                last_end_line = line[:match.end()]
    if last_end_idx != -1:
        lines = lines[:last_end_idx + 1]
        lines[-1] = last_end_line

    return '\n'.join(lines).strip()


def read_ppl_file(path: str) -> str:
    with open(path, 'rb') as f:
        raw = f.read()

    if _looks_like_utf16_hp_file(raw[:200]):
        start = _find_utf16_payload_start(raw)
        decoded = raw[start:].decode('utf-16-le', errors='ignore')
        return _trim_to_program_bounds(_sanitize_text(decoded))

    text = raw.decode('utf-8', errors='replace')
    return _trim_to_program_bounds(_sanitize_text(text))


def read_ppl_if_exists(path: str) -> str:
    if not os.path.exists(path):
        raise FileNotFoundError(path)
    return read_ppl_file(path)
