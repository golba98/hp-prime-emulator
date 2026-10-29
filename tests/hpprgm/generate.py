#!/usr/bin/env python3
"""Reproduce authored fixtures using independently stated firmware object layout.
No vendor files, downloaded sources, device access or Rust serializer involved.
"""
import hashlib
import json
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parent
cases = []


def word(n):
    return struct.pack('<I', n)


def obj(ty, payload):
    return word(4 + len(payload)) + word(ty) + payload


def group(*objects):
    payload = b''.join(objects)
    return word(len(payload)) + payload


def name64(s):
    b = (s + '\0').encode('utf-16le')
    assert len(b) <= 64
    return b.ljust(64, b'\0')


def container(source, *, unedited=False, cache=False, unknown=False):
    b = b''.join(map(word, [0xB28A617C, 0xFFFFFFFE, 0]))
    b += b''.join(obj(ty, word(0)) for ty in [0x007FFF05, 0x023FFF05, 0x00BFFF05])
    if cache:
        b += obj(0x0100023E, group(obj(0x0040020B, name64('CACHED')), obj(0x00800205, word(9))))
    parts = [obj(0x0040008B, name64('Main')), obj(0x00800085, word(0))]
    if not unedited:
        parts.append(obj(0x00C0009B, (source + '\0').encode('utf-16le')))
    b += obj(0x014000BE, group(*parts))
    b += obj(0x0240028E, b''.join(group(obj(0x0040028B, bytes(64)), obj(0x00800284, b'\xff' * 8)) for _ in range(10)))
    if unknown:
        b += obj(0x03400005, word(123456))
    return b


def add(ident, source=None, *, data=None, path=None, kind='container', encoding=None,
        name=None, source_mode=False, import_result='ok', firmware=False):
    # NUL is a reserved device name on Windows, but remains the stable case ID.
    path = path or f'files/{"NUL_BYTES" if ident == "NUL" else ident}.hpprgm'
    if data is None:
        data = container(source) if kind == 'container' else source.encode('utf-8')
    p = ROOT / path
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_bytes(data)
    c = dict(id=ident, path=path, sha256=hashlib.sha256(data).hexdigest(),
             size=len(data), source_file=f'sources/{ident}.ppl' if source is not None else None, container=kind == 'container',
             encoding=encoding or ('utf-16le-container' if kind == 'container' else 'utf-8'),
             name=name, as_source=source_mode, import_result=import_result, firmware=firmware)
    if source is not None:
        golden = ROOT / c['source_file']
        golden.parent.mkdir(exist_ok=True)
        golden.write_bytes(source.encode('utf-8'))
    cases.append(c)


def program(name, body='RETURN 1;'):
    return f'EXPORT {name}()\nBEGIN\n {body}\nEND;\n'


for ident, body in [
    ('MINIMAL', 'RETURN 1;'),
    ('NESTED', 'LOCAL i,s:=0; FOR i FROM 1 TO 3 DO IF i>1 THEN s:=s+i; END; END; RETURN s;'),
    ('UNICODE', 'PRINT("≥ → é Σ π 日本語");'),
    ('SUPPLEMENT', 'PRINT("emoji 😀 and 𝛑");'),
    ('GRAPHICS', 'RECT_P(0,0,20,20,#FF0000h);'),
    ('WHITESPACE', '// comment\n\tRETURN 1;  '),
    ('ESCAPES', 'PRINT("path \\\\ and quote \\" end");'),
]:
    add(ident, program(ident, body), firmware=ident != 'ESCAPES')
add('MULTI', 'HELPER(x)\nBEGIN\n RETURN x*x;\nEND;\n' + program('MULTI', 'RETURN HELPER(3);'), firmware=True)
add('ENTRIES', program('ENTRIES') + program('SECOND', 'RETURN 2;'), firmware=True)
add('LONGIDENT', program('LONGIDENT', 'LOCAL ' + 'v'*64 + ':=1; RETURN ' + 'v'*64 + ';'), firmware=True)
add('LOWER', program('LOWER'), path='files/lower.hpprgm', firmware=True)
add('UPPEREXT', program('UPPEREXT'), path='files/UPPEREXT.HPPRGM', firmware=True)
add('SPACEFILE', program('SPACEFILE'), path='files/My Program.hpprgm', name='SPACEFILE', firmware=True)
add('UNICODEPATH', program('UNICODEPATH'), path='files/dossier é 日本/UNICODEPATH.hpprgm', firmware=True)
add('BADFILENAME', program('BADFILENAME'), path='files/Bad Name.hpprgm', import_result='invalid-name')
add('BADUNICODENAME', program('BADUNICODENAME'), path='files/Éxample.hpprgm', import_result='invalid-name')
add('LEADINGDIGIT', program('LEADINGDIGIT'), name='1BAD', import_result='invalid-name')
add('EMPTYNAME', program('EMPTYNAME'), name='', import_result='invalid-name')
add('NAME31', program('A'*31), path='files/'+'A'*31+'.hpprgm', firmware=True)
add('NAME32', program('NAME32'), path='files/'+'A'*32+'.hpprgm', import_result='invalid-name')
add('UNEDITED', '', data=container('', unedited=True), firmware=True)
add('CACHED', program('CACHED'), data=container(program('CACHED'), cache=True), firmware=True)
add('UNKNOWN', program('UNKNOWN'), data=container(program('UNKNOWN'), unknown=True))
add('CONTAINER_CRLF', program('CONTAINER_CRLF').replace('\n','\r\n'), firmware=True)
add('LARGE', program('LARGE', '// ' + 'x'*65536 + '\n RETURN 7;'), firmware=True)
add('LARGE256', program('LARGE256', '// ' + 'x'*262144 + '\n RETURN 9;'), firmware=True)
for ident, encoding, bom, line in [
    ('LF', 'utf-8', b'', '\n'), ('CRLF', 'utf-8', b'', '\r\n'),
    ('CR', 'utf-8', b'', '\r'), ('UTF8BOM', 'utf-8', b'\xef\xbb\xbf', '\n'),
    ('UTF16LE', 'utf-16le', b'\xff\xfe', '\n'), ('UTF16BE', 'utf-16be', b'\xfe\xff', '\r\n'),
]:
    src = program(ident, 'PRINT("≥ é 😀");').replace('\n',line)
    add(ident, src, data=bom+src.encode(encoding), kind='text', encoding=encoding, source_mode=True, firmware=True)
add('MIXED', 'EXPORT MIXED()\r\nBEGIN\r RETURN 1;\nEND;\r\n', kind='text', source_mode=True, firmware=True)
add('TEXTDEFAULT', program('TEXTDEFAULT'), kind='text', import_result='plain-text')
add('EMPTY', '', data=b'', kind='text', source_mode=True, import_result='source')
add('SPACEONLY', ' \n\t', kind='text', source_mode=True, import_result='source')
for ident, data in [('BADUTF8', b'EXPORT BAD() BEGIN PRINT("\xc3("); END;'),
                    ('ODDUTF16', b'\xff\xfeA'), ('SURROGATE', b'\xff\xfe\x00\xd8'),
                    ('NUL', b'EXPORT X()\0TAIL'), ('BOMLESS16', program('BOMLESS16').encode('utf-16le'))]:
    add(ident, data=data, kind='invalid', source_mode=True, import_result='source')
good = container(program('BROKEN'))
for ident, data in [('TRUNCATED',good[:-7]), ('HEADERONLY', good[:12]), ('BADPREAMBLE',good[:4]+bytes(4)+good[8:]),
                    ('TRAILING',good+b'\1\2\3'), ('BADLENGTH',good[:48]+word(0xffffffff)+good[52:])]:
    add(ident, data=data, kind='invalid', import_result='container')
# Partial list group header followed by bytes outside the list: used to panic.
add('PARTIALGROUP', data=good[:48]+word(5)+word(0x014000BE)+bytes(4), kind='invalid', import_result='container')
# Source with a lone UTF-16 surrogate; structurally valid container.
bad = container('X')
at = bad.index(obj(0x00C0009B, 'X\0'.encode('utf-16le')))
bad = bad[:at]+obj(0x00C0009B,b'\x00\xd8\0\0')+bad[at+12:]
add('BADSOURCE16', data=bad, kind='invalid', import_result='container')
# Duplicate source section and unsupported multi-source group, without data loss.
files_size=4+struct.unpack_from('<I',good,48)[0]
add('DUPSECTION', data=good+good[48:48+files_size], kind='invalid', import_result='container')
# Deep metadata exercises the explicit host parser resource bound.
deep=obj(0x77770005,word(0))
for _ in range(140): deep=obj(0x7777000E,group(deep))
add('DEEP',data=good+deep,kind='invalid',import_result='container')
(ROOT/'cases.json').write_text(json.dumps(cases,ensure_ascii=False,indent=2)+'\n')
print(f'{len(cases)} fixtures, {sum(c["firmware"] for c in cases)} firmware candidates')
