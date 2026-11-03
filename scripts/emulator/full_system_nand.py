#!/usr/bin/env python3
"""Bounded synthetic NAND qtest and real ARM926 transactions/boot handoff.

Never accepts proprietary flash inputs. Generates complete erased main+OOB media
and a disposable qcow2 overlay. Original resources and USB are never opened.
"""
import argparse
import hashlib
from contextlib import contextmanager
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import zlib

from full_system_preflight import inventory, PIN
from full_system_run import checked_manifest, digest, qmp_command, wait_socket
from full_system_intc import QTest, reset_event

CASES = {'nand': 1, 'nand_ecc': 2, 'boot_chain': 3, 'nand_model': 0}
BASE = 0x4e000000
PAGE = 2048
OOB = 64
PAGES = 131072
IMAGE_SIZE = PAGES * (PAGE + OOB)
MASK = 0xf0ffffff


def main_ecc(data):
    """Independent bit-parity equations, without the QEMU lookup/digester."""
    if not 0 < len(data) <= PAGE:
        raise ValueError('ECC input outside supported MAIN bounds')
    parity = 0
    for byte, value in enumerate(data):
        for bit in range(8):
            if value & (1 << bit):
                for dimension in range(11):
                    pair = 2 * dimension if dimension < 9 else 28 + 2 * (dimension - 9)
                    parity ^= 1 << (pair + (0 if byte & (1 << dimension) else 1))
                for dimension in range(3):
                    parity ^= 1 << (18 + 2 * dimension + ((bit >> dimension) & 1))
    return (~parity) & 0xffffffff


def image_bytes(elf):
    """Extract bounded ARM ELF load segments without invoking host objcopy."""
    if Path(elf).stat().st_size > 1024 * 1024:
        raise ValueError('guest ELF exceeds bounded artifact size')
    data = Path(elf).read_bytes()
    if len(data) < 52 or data[:6] != b'\x7fELF\x01\x01' or struct.unpack_from('<HH', data, 16) != (2, 40):
        raise ValueError('expected little-endian ARM ELF32 executable')
    entry, phoff = struct.unpack_from('<II', data, 24)
    phsize, count = struct.unpack_from('<HH', data, 42)
    if phsize < 32 or not 1 <= count <= 64 or phoff + phsize * count > len(data):
        raise ValueError('ELF program headers outside artifact')
    segments = []
    for i in range(count):
        kind, offset, _, physical, filesz, memsz = struct.unpack_from('<6I', data, phoff + i * phsize)
        if kind == 1 and memsz:
            if filesz > memsz or offset + filesz > len(data):
                raise ValueError('invalid load segment bounds')
            segments.append((physical, data[offset:offset + filesz], memsz))
    if not segments:
        raise ValueError('ELF has no load segments')
    start = min(s[0] for s in segments)
    end = max(s[0] + s[2] for s in segments)
    if not start <= entry < end or end - start > 8192:
        raise ValueError('guest entry or image span outside bounds')
    binary = bytearray(end - start)
    for address, payload, _ in segments:
        binary[address - start:address - start + len(payload)] = payload
    return entry, bytes(binary)


def build_guest(output, case):
    output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve().parents[2] / 'research/full-system'
    tools = inventory(Path('/nonexistent'), [])['tools']
    cc = tools['clang'] or tools['arm-none-eabi-gcc']
    ld = tools['ld.lld'] or tools['rust-lld']
    if not cc or not ld:
        raise ValueError('ARM compiler and linker required')
    target = ['--target=armv5-none-eabi'] if tools['clang'] else []
    common = [cc, *target, '-mcpu=arm926ej-s', '-marm', '-g', '-ffreestanding',
              '-fno-builtin', '-fno-stack-protector', '-Os']
    flavor = ['-flavor', 'gnu'] if ld == tools['rust-lld'] else []
    stage = b''
    flags = ['-DCASE=' + str(CASES[case])]
    if case == 'boot_chain':
        subprocess.run([*common, '-c', str(source/'nand_stage2.S'), '-o', str(output/'stage2.o')], check=True, timeout=30)
        subprocess.run([ld, *flavor, '-m', 'armelf', '-T', str(source/'nand_stage2.ld'),
                        str(output/'stage2.o'), '-o', str(output/'stage2.elf')], check=True, timeout=30)
        entry, stage = image_bytes(output/'stage2.elf')
        if entry != 0x30010000 or len(stage) > PAGE:
            raise ValueError('stage2 image outside single NAND page')
        flags += [f'-DSTAGE_SIZE={len(stage)}', f'-DSTAGE_CRC={zlib.crc32(stage)}u']
    for name in ['nand_start.S', 'nand_guest.c']:
        subprocess.run([*common, *flags, '-c', str(source/name), '-o', str(output/(name+'.o'))], check=True, timeout=30)
    linkflags = ['--defsym=BOOT_STAGE=1'] if case == 'boot_chain' else []
    elf = output/'nand.elf'
    subprocess.run([ld, *flavor, '-m', 'armelf', *linkflags, '-T', str(source/'nand.ld'),
                    str(output/'nand_start.S.o'), str(output/'nand_guest.c.o'), '-o', str(elf)], check=True, timeout=30)
    entry, binary = image_bytes(elf)
    if entry != (0 if case == 'boot_chain' else 0x30000000):
        raise ValueError('unexpected guest entry')
    meta = {'path': str(elf), 'sha256': digest(elf), 'entry': hex(entry),
            'source_sha256': {name: digest(source/name) for name in
                              ['nand_start.S', 'nand_guest.c', 'nand.ld', 'nand_stage2.S', 'nand_stage2.ld']},
            'linked': True, 'executed': False,
            'compiler': {'path': cc, 'sha256': digest(Path(cc).resolve()),
                         'version': subprocess.check_output([cc, '--version'], text=True, timeout=10).splitlines()[0]},
            'linker': {'path': ld, 'sha256': digest(Path(ld).resolve()),
                       'version': subprocess.check_output([ld, *flavor, '--version'], text=True, timeout=10).splitlines()[0]}}
    if stage:
        meta['stage2_sha256'] = digest(output/'stage2.elf')
        meta['stage2_crc32'] = zlib.crc32(stage)
        meta['stage2_size'] = len(stage)
    return meta, binary, stage


def create_image(output, bootstrap=b'', stage=b''):
    base = output/'base.raw'
    # Sparse holes read as00; erased NAND must instead contain explicit FF.
    with base.open('xb') as stream:
        block = b'\xff' * (1024 * 1024)
        remaining = IMAGE_SIZE
        while remaining:
            stream.write(block[:min(len(block), remaining)])
            remaining -= min(len(block), remaining)
    if bootstrap or stage:
        with base.open('r+b') as stream:
            for offset in range(0, len(bootstrap), PAGE):
                stream.seek((offset // PAGE) * (PAGE + OOB))
                stream.write(bootstrap[offset:offset + PAGE])
            stream.seek(4 * (PAGE + OOB))
            stream.write(stage)
    before = digest(base)
    subprocess.run(['qemu-img', 'create', '-f', 'qcow2', '-o', 'compat=0.10', '-F', 'raw',
                    '-b', 'base.raw', 'flash.qcow2'], cwd=output, stdout=subprocess.DEVNULL, check=True, timeout=30)
    return {'format': 'interleaved_main_oob', 'bytes': IMAGE_SIZE, 'page_bytes': PAGE,
            'oob_bytes': OOB, 'pages_per_block': 64, 'pages': PAGES,
            'base_sha256': before, 'overlay_format': 'qcow2_v2', 'disposable': True}


@contextmanager
def machine(root, output, name, *, guest=False, boot=False, readonly=False):
    meta = checked_manifest(root)
    logs = output/name
    logs.mkdir(exist_ok=False)
    sockets = Path(tempfile.mkdtemp(prefix='prime-nand-', dir=Path.home()/'.cache'))
    cid = logs/'container.cid'
    args = ['podman', 'run', '--rm', '--name', sockets.name, '--cidfile', str(cid), '--network', 'none',
            '--memory', '1g', '--ulimit', 'fsize=8388608:8388608', '--userns', 'keep-id',
            '--user', f'{os.getuid()}:{os.getgid()}', '--read-only', '--tmpfs', '/tmp:rw,size=32m',
            '-v', f'{root/"build"}:/build:ro,z', '-v', f'{output}:/run:rw,Z',
            '-v', f'{output/"base.raw"}:/run/base.raw:ro,Z', '-v', f'{sockets}:/sockets:rw,Z',
            meta['image'], '/build/arm-softmmu/qemu-system-arm',
            '-M', 'hp-prime' + ('' if guest else ',accel=qtest'), '-display', 'none',
            '-monitor', 'none', '-serial', f'file:/run/{name}/uart.log', '-net', 'none',
            '-drive', 'if=mtd,index=0,format=' + ('raw,file=/run/base.raw,readonly=on' if readonly else 'qcow2,file=/run/flash.qcow2'),
            '-qmp', 'unix:/sockets/qmp.sock,server,nowait', '-d', ('in_asm,guest_errors,unimp' if guest else 'guest_errors,unimp'), '-D', f'/run/{name}/model.log']
    if guest:
        args += ['-S', '-gdb', 'unix:/sockets/gdb.sock,server,nowait']
        if not boot:
            args += ['-kernel', '/run/guest/nand.elf']
    else:
        args += ['-qtest', 'unix:/sockets/qtest.sock,server,nowait', '-qtest-log', f'/run/{name}/qtest.log']
    (logs/'launch.json').write_text(json.dumps(args, indent=2)+'\n')
    process = None
    records = []
    try:
        with (logs/'stdout').open('xb') as stdout, (logs/'stderr').open('xb') as stderr:
            process = subprocess.Popen(args, stdout=stdout, stderr=stderr, start_new_session=True)
            wait_socket(sockets/('gdb.sock' if guest else 'qtest.sock'), process)
            wait_socket(sockets/'qmp.sock', process)
            with socket.socket(socket.AF_UNIX) as msock:
                msock.settimeout(5); msock.connect(str(sockets/'qmp.sock'))
                with msock.makefile('rwb') as mstream:
                    records.append({'receive': json.loads(mstream.readline(65536))})
                    qmp_command(mstream, 'qmp_capabilities', records)
                    if guest:
                        yield sockets, mstream, records
                    else:
                        with socket.socket(socket.AF_UNIX) as qs:
                            qs.settimeout(5); qs.connect(str(sockets/'qtest.sock'))
                            with qs.makefile('rwb') as stream:
                                test = QTest(stream)
                                try:
                                    yield test, mstream, records
                                finally:
                                    (logs/'commands.json').write_text(json.dumps(test.records, indent=2)+'\n')
                    qmp_command(mstream, 'quit', records)
            if process.wait(timeout=10) != 0:
                raise RuntimeError('NAND QEMU exited unsuccessfully')
    finally:
        if cid.is_file():
            identity = cid.read_text().strip()
            if len(identity) == 64 and all(c in '0123456789abcdef' for c in identity):
                subprocess.run(['podman', 'rm', '--force', identity], stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, timeout=10, check=False)
        if process is not None and process.poll() is None:
            process.kill(); process.wait(timeout=10)
        shutil.rmtree(sockets)
        (logs/'qmp.json').write_text(json.dumps(records, indent=2)+'\n')


def byte_read(test):
    return int(test.command(f'readb {BASE+16:#x}').split()[1], 0)


def byte_write(test, off, value):
    test.command(f'writeb {BASE+off:#x} {value:#x}')


def address(test, row, column=0):
    for value in [column & 255, column >> 8, row & 255, (row >> 8) & 255, row >> 16]:
        byte_write(test, 12, value)


def read_page(test, row, column, count):
    byte_write(test, 8, 0); address(test, row, column); byte_write(test, 8, 0x30)
    if test.read(BASE+40) & 1:
        raise RuntimeError('READ never entered modeled busy state')
    test.command('clock_step 1000')
    if not test.read(BASE+40) & 1:
        raise RuntimeError('READ did not complete after modeled deadline')
    return bytes(byte_read(test) for _ in range(count))


def program(test, row, column, data, complete=True):
    byte_write(test, 8, 0x80); address(test, row, column)
    for value in data: byte_write(test, 16, value)
    byte_write(test, 8, 0x10)
    if complete:
        test.command('clock_step 200000')
        byte_write(test, 8, 0x70)
        status = byte_read(test)
        if status & 1:
            raise RuntimeError(f'program failed status{status:#x}, row{row}, column{column}')


def erase(test, row):
    byte_write(test, 8, 0x60)
    for value in [row & 255, (row >> 8) & 255, row >> 16]: byte_write(test, 12, value)
    byte_write(test, 8, 0xd0); test.command('clock_step 2000000')


def host_tests(root, output):
    summary = []
    payload = bytes((i * 37 + 11) & 255 for i in range(PAGE+OOB))
    with machine(root, output, 'host') as (test, qmp, records):
        if test.read(BASE+4) != 0x100c6 or test.read(BASE+40) & 0x7f != 0x1d:
            raise RuntimeError('controller reset defaults differ')
        test.write(BASE+4, 0xc1)
        if read_page(test, 193, 0, PAGE+OOB) != b'\xff'*(PAGE+OOB):
            raise RuntimeError('synthetic NAND not erased')
        program(test, 193, 0, payload)
        if read_page(test, 193, 0, PAGE+OOB) != payload:
            raise RuntimeError('page/OOB round trip differs')
        program(test, 193, 0, b'\xff'*16)
        if read_page(test, 193, 0, 16) != payload[:16]:
            raise RuntimeError('program incorrectly changed0to1')
        if test.read(BASE+32) != 0:
            raise RuntimeError('NFDATA write corrupted NFSBLK')
        summary += ['erased_ff', 'page_oob_roundtrip', 'program_1_to_0', 'nfdata_no_fallthrough']
        # Explicit first-byte spare markers; no chip-side auto-skip/failure policy.
        program(test, 256, PAGE, b'\x00')
        if read_page(test, 256, PAGE, 1) != b'\x00' or read_page(test, 257, PAGE, 1) != b'\xff':
            raise RuntimeError('bad-block marker physical addressing differs')
        summary.append('physical_oob_badblock_marker')
        erase(test, 193)
        if read_page(test, 193, 0, PAGE+OOB) != b'\xff'*(PAGE+OOB):
            raise RuntimeError('erase did not clear main and spare')
        program(test, PAGES-1, PAGE+OOB-1, b'\x5a')
        if read_page(test, PAGES-1, PAGE+OOB-1, 1) != b'\x5a':
            raise RuntimeError('last flash byte differs')
        summary += ['erase_main_oob', 'last_physical_byte']
        # Invalid row/column, excessive cycles and confirm without setup fail safely.
        for row, col in [(PAGES, 0), (1, PAGE+OOB), (0xffffff, 0xffff)]:
            byte_write(test, 8, 0); address(test, row, col); byte_write(test, 8, 0x30)
            byte_write(test, 8, 0x70)
            if not byte_read(test) & 1: raise RuntimeError('invalid address succeeded')
        byte_write(test, 8, 0x80)
        for _ in range(12): byte_write(test, 12, 0xff)
        byte_write(test, 8, 0x10); byte_write(test, 8, 0x70)
        if not byte_read(test) & 1: raise RuntimeError('malformed program succeeded')
        byte_write(test, 8, 0xff)
        program(test, 194, 0, b'\x00'*16, complete=False)
        byte_write(test, 8, 0xff); test.command('clock_step 200000')
        if read_page(test, 194, 0, 16) != b'\xff'*16:
            raise RuntimeError('flash reset committed pending program')
        summary += ['malformed_sequence_bounds', 'flash_reset_cancels_program']
        program(test, 195, 0, b'\x00'*16, complete=False)
        reset_event(qmp, records); test.command('clock_step 200000')
        if test.read(BASE+4) != 0x100c6: raise RuntimeError('machine reset retained controller state')
        test.write(BASE+4, 0xc1)
        if read_page(test, 195, 0, 16) != b'\xff'*16: raise RuntimeError('machine reset committed pending program')
        test.write(BASE+32, 3); test.write(BASE+36, 5); test.write(BASE+4, 0x200c1)
        test.write(BASE+32, 8); test.write(BASE+4, 0xc1)
        if test.read(BASE+32) != 3 or not test.read(BASE+4)&0x20000:
            raise RuntimeError('lock-tight did not persist until reset')
        reset_event(qmp, records); test.write(BASE+4, 0xc1)
        summary += ['machine_reset_cancellation', 'lock_tight_until_reset']
        vectors = [b'\xff'*512, b'\x00'*512, bytes((i*37+11)&255 for i in range(512)),
                   hashlib.shake_256(b'M19 NAND ECC').digest(512), bytes([1])+b'\x00'*511, b'\x00'*511+bytes([128])]
        for data in vectors:
            byte_write(test, 8, 0x80); address(test, 196)
            test.write(BASE+4, 0x61)
            for value in data: byte_write(test, 16, value)
            test.write(BASE+4, 0xc1)
            observed = test.read(BASE+52)
            if observed != main_ecc(data):
                raise RuntimeError(f'independent ECC reference differs {observed:#x}!={main_ecc(data):#x}')
            byte_write(test, 8, 0xff)
        summary.append('independent_main512_ecc_vectors')
        # Mixed bus widths issue2/4 ordered byte cycles, not1.
        byte_write(test, 8, 0x80); address(test, 197)
        test.write(BASE+16, 0x44332211)
        test.command(f'writew {BASE+16:#x} 0x6655')
        byte_write(test, 8, 0x10); test.command('clock_step 200000')
        if read_page(test, 197, 0, 6) != b'\x11\x22\x33\x44\x55\x66':
            raise RuntimeError('NFDATA bus width sequencing differs')
        program(test, 198, PAGE, b'persist_oob')
        summary.append('halfword_word_byte_cycles')
    with machine(root, output, 'reopen') as (test, _, _):
        test.write(BASE+4, 0xc1)
        if read_page(test, 198, PAGE, 11) != b'persist_oob' or read_page(test, PAGES-1, PAGE+OOB-1, 1) != b'\x5a':
            raise RuntimeError('fresh-process OOB/data persistence differs')
        summary.append('fresh_process_persistence')
    with machine(root, output, 'readonly', readonly=True) as (test, _, _):
        test.write(BASE+4, 0xc1)
        program(test, 193, 0, b'\x00', complete=False)
        byte_write(test, 8, 0x70)
        if not byte_read(test)&1 or read_page(test, 193, 0, 1) != b'\xff':
            raise RuntimeError('read-only media accepted writes')
        summary.append('readonly_refuses_writes')
    return summary


def validate_observation(data, case, uart, guest):
    if case not in ('nand', 'nand_ecc', 'boot_chain'):
        raise ValueError('unsupported guest observation')
    expected_entry = 0 if case == 'boot_chain' else 0x30000000
    words = data.get('mailbox', [])
    if data.get('initial_pc') != expected_entry or data.get('initial_mailbox') != 0:
        raise ValueError('guest entry/mailbox not fresh')
    if data.get('stepped_pc') != expected_entry+4 or data.get('stopped_pc') != data.get('halt_address'):
        raise ValueError('guest instructions or terminal PC not observed')
    if not isinstance(words, list) or len(words) != 64 or any(type(v) is not int or not 0 <= v <= 0xffffffff for v in words):
        raise ValueError('mailbox requires64 bounded integer words')
    halt_start = 0x30010000 if case == 'boot_chain' else 0x30000000
    if not halt_start <= data.get('halt_address', 0) < halt_start + 8192:
        raise ValueError('terminal address outside guest code')
    if data.get('cpsr', 0) & 0x3f != 0x13:
        raise ValueError('guest must complete in ARM SVC mode')
    if words[0] != 0x600d0019:
        raise ValueError('NAND guest reported failure/incomplete execution')
    if case == 'nand':
        expected_sum = sum(((i+p)*37+11)&255 for p in range(3) for i in range(PAGE+OOB))
        if words[2:7] != [0xec, 0xda, 0x10, 0x95, 0x44] or words[8:11] != [expected_sum, 3, PAGE+OOB] or words[11:14] != [0,320,255] or uart != b'M19 NAND\n':
            raise ValueError('guest NAND/OOB observations differ')
    elif case == 'nand_ecc':
        pattern = bytes((((i*37+11)&255) ^ (4 if i == 201 else 0)) for i in range(512))
        if (words[2] != main_ecc(pattern) or words[3] != 0 or words[4] != 1|(1<<4)|(123<<7)
                or words[5] != sum(pattern) or words[6] != 3 or words[7] != 2 or uart != b'M19 ECC\n'):
            raise ValueError('guest ECC reference/classification/software correction differs')
    elif case == 'boot_chain':
        if words[2:6] != [guest['stage2_crc32'], guest['stage2_size'], 0xface0019, 0x5a190002] or uart != b'M19 BOOT2\n':
            raise ValueError('guest verified stage2 handoff/marker differs')
    data['uart_hex'] = uart.hex()
    return data


def run(root, output, case, timeout=30):
    if case not in CASES or not 1 <= timeout <= 120:
        raise ValueError('unsupported case or deadline outside1..120seconds')
    root, output = root.resolve(), output.resolve()
    meta = checked_manifest(root)
    output.mkdir(parents=True, exist_ok=False)
    image = subprocess.check_output(['podman', 'image', 'inspect', meta['image'], '--format', '{{.Id}}'], text=True, timeout=10).strip()
    if image != meta['image_id']:
        raise ValueError('runtime image changed')
    report = {'schema_version': 1, 'scope': 'full_system_synthetic_nand', 'case': case, 'status': 'failed',
              'machine': 'hp-prime', 'cpu': 'arm926', 'reference_revision': PIN,
              'binary_sha256': meta['binary_sha256'], 'image_id': image, 'patches': meta.get('patches', []),
              'kernel_boot_verified': False, 'physical_hardware_verified': False,
              'guest_cpu_execution_verified': False, 'host_device_tests_verified': False}
    try:
        guest, bootstrap, stage = build_guest(output/'guest', case if case != 'nand_model' else 'nand')
        report['guest'] = guest
        report['flash'] = create_image(output, bootstrap if case == 'boot_chain' else b'', stage)
        if case in ('nand', 'nand_model'):
            report['host_checks'] = host_tests(root, output)
            report['host_device_tests_verified'] = True
        if case == 'boot_chain':
            with machine(root, output, 'boot_reset') as (test, stream, records):
                expected = struct.unpack_from('<I', bootstrap, 4096)[0]
                if test.read(4096) != expected:
                    raise RuntimeError('bootstrap copied OOB or wrong main page')
                test.write(4096, 0)
                reset_event(stream, records)
                if test.read(4096) != expected:
                    raise RuntimeError('reset did not restore main-only bootstrap')
            report['host_checks'] = ['bootstrap_skips_oob', 'bootstrap_reconstructed_at_reset']
            report['host_device_tests_verified'] = True
        if case != 'nand_model':
            with machine(root, output, 'guest_run', guest=True, boot=case == 'boot_chain') as (sockets, _, _):
                halt = 'stage2_halt' if case == 'boot_chain' else 'nand_halt'
                elf = output/'guest'/('stage2.elf' if case == 'boot_chain' else 'nand.elf')
                script = '''set pagination off
set confirm off
set auto-load off
set architecture arm
file {elf}
target remote {socket}
python
import gdb,json
obs={{}}
obs['initial_pc']=int(gdb.parse_and_eval('$pc'))
obs['initial_mailbox']=int(gdb.parse_and_eval('*(unsigned int*)0x30001000'))
gdb.execute('stepi')
obs['stepped_pc']=int(gdb.parse_and_eval('$pc'))
gdb.Breakpoint('{halt}',type=gdb.BP_HARDWARE_BREAKPOINT)
gdb.execute('continue')
obs['stopped_pc']=int(gdb.parse_and_eval('$pc'))
obs['halt_address']=int(gdb.parse_and_eval('&{halt}'))
obs['mailbox']=[int(gdb.parse_and_eval('*(unsigned int*)%d'%(0x30001000+4*i))) for i in range(64)]
obs['registers']={{'r%d'%i:int(gdb.parse_and_eval('$r%d'%i)) &0xffffffff for i in range(16)}}
obs['cpsr']=int(gdb.parse_and_eval('$cpsr'))
with open({result},'x') as stream: json.dump(obs,stream,indent=2)
end
quit
'''.format(elf=json.dumps(str(elf)), socket=str(sockets/'gdb.sock'), halt=halt, result=repr(str(output/'observations.json')))
                (output/'debug.gdb').write_text(script)
                with (output/'guest_run/gdb.log').open('xb') as log:
                    subprocess.run(['gdb', '--nx', '--nh', '--batch', '-x', str(output/'debug.gdb')],
                                   stdout=log, stderr=subprocess.STDOUT, check=True, timeout=timeout)
                report['observations'] = validate_observation(json.loads((output/'observations.json').read_text()), case,
                                                             (output/'guest_run/uart.log').read_bytes(), guest)
            report['guest_cpu_execution_verified'] = True
            guest['executed'] = True
        report['flash']['base_after_sha256'] = digest(output/'base.raw')
        if report['flash']['base_after_sha256'] != report['flash']['base_sha256']:
            raise RuntimeError('read-only base identity changed')
        report['status'] = 'passed'
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report['error'] = str(error)
    finally:
        report['artifacts'] = {str(path.relative_to(output)): digest(path) for path in sorted(output.rglob('*'))
                               if path.is_file() and path.name in ('model.log', 'uart.log', 'gdb.log', 'qmp.json', 'launch.json', 'flash.qcow2')}
        (output/'result.json').write_text(json.dumps(report, indent=2)+'\n')
    return report


def main():
    def cancelled(_signal, _frame):
        raise InterruptedError('NAND experiment cancelled')
    signal.signal(signal.SIGTERM, cancelled)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--case', choices=CASES, required=True)
    parser.add_argument('--timeout', type=int, default=30)
    args = parser.parse_args()
    try:
        result = run(args.build_root, args.output, args.case, args.timeout)
        print(json.dumps(result, indent=2))
        return 0 if result['status'] == 'passed' else 1
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(json.dumps({'status': 'failed', 'error': str(error)})); return 2


if __name__ == '__main__':
    raise SystemExit(main())
