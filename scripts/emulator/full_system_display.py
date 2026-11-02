#!/usr/bin/env python3
"""Execute original ARM926 LCD and matrix/I2C controls on hp-prime.

All display pixels and input samples are produced by executed guest instructions.
qtest only exchanges commands in the guest mailbox and observes its results.
No proprietary code, physical calculator or production HLE is involved.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import tempfile
import time
import zlib

from full_system_preflight import inventory, PIN
from full_system_run import checked_manifest, qmp_command, wait_socket, digest
from full_system_intc import QTest, reset_event

MAILBOX = 0x30001000
CASES = ('lcd', 'input')


def build_guest(output):
    output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve().parents[2] / 'research/full-system'
    tools = inventory(Path('/nonexistent'), [])['tools']
    compiler = tools['clang'] or tools['arm-none-eabi-gcc']
    linker = tools['ld.lld'] or tools['rust-lld']
    if not compiler or not linker:
        raise ValueError('display control requires an ARM compiler/linker')
    target = ['--target=armv5-none-eabi'] if tools['clang'] else []
    for name in ('display.S', 'display.c'):
        subprocess.run([compiler, *target, '-mcpu=arm926ej-s', '-marm', '-O1',
                        '-ffreestanding', '-fno-builtin', '-g', '-c', str(source/name),
                        '-o', str(output/(name+'.o'))], check=True, timeout=30)
    flavor = ['-flavor', 'gnu'] if linker == tools['rust-lld'] else []
    elf = output/'display.elf'
    subprocess.run([linker, *flavor, '-m', 'armelf', '-T', str(source/'display.ld'),
                    str(output/'display.S.o'), str(output/'display.c.o'), '-o', str(elf)],
                   check=True, timeout=30)
    data = elf.read_bytes()
    if (data[:6] != b'\x7fELF\x01\x01' or struct.unpack_from('<HH', data, 16) != (2, 40)
            or struct.unpack_from('<I', data, 24)[0] != 0x30000000):
        raise ValueError('unexpected display ELF identity/entry')
    return {'path': str(elf), 'sha256': digest(elf), 'entry': '0x30000000',
            'sources': {n: digest(source/n) for n in ('display.S', 'display.c', 'display.ld')},
            'linked': True, 'executed': False}


def expected_frame(second=False):
    """Independent image oracle: coordinate RGB ramps and a single update."""
    pixels = bytearray()
    for y in range(240):
        for x in range(320):
            r, g, b = (x//8) % 32, (y//4) % 64, ((x ^ y)//8) % 32
            if second:
                r, g, b = 31-r, 63-g, 31-b
                if (x, y) == (17, 23):
                    r, g, b = 31, 0, 31
            pixels.extend(((r*8)+(r//4), (g*4)+(g//16), (b*8)+(b//4)))
    return bytes(pixels)


def validate_ppm(path, second=False):
    data = path.read_bytes()
    fields = data.split(b'\n', 3)
    if len(fields) != 4 or fields[:3] != [b'P6', b'320 240', b'255']:
        raise ValueError('capture is not an exact 320x240 RGB PPM')
    pixels = fields[3]
    expected = expected_frame(second)
    if pixels != expected:
        wrong = sum(a != b for a, b in zip(pixels, expected)) + abs(len(pixels)-len(expected))
        raise ValueError(f'guest LCD capture differs in {wrong} RGB bytes')
    return pixels


def write_png(path, pixels):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind+data))
    rows = b''.join(b'\0' + pixels[y*960:(y+1)*960] for y in range(240))
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 320, 240, 8, 2, 0, 0, 0))
                     + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def wait_word(test, address, expected, deadline):
    while time.monotonic() < deadline:
        value = test.read(address)
        if value == expected:
            return
        time.sleep(.01)
    raise TimeoutError(f'guest mailbox {address:#x}: expected {expected:#x}, got {value:#x}')


def guest_command(test, command, deadline):
    test.write(MAILBOX+4, command)
    wait_word(test, MAILBOX+8, command, deadline)


def key(qmp, records, code, down):
    qmp_command(qmp, 'input-send-event', records, {'events': [
        {'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': code}}}]})


def touch(qmp, records, x, y, down):
    qmp_command(qmp, 'input-send-event', records, {'events': [
        {'type': 'abs', 'data': {'axis': 'x', 'value': x}},
        {'type': 'abs', 'data': {'axis': 'y', 'value': y}},
        {'type': 'btn', 'data': {'button': 'left', 'down': down}}]})


def sample(test, command, deadline):
    guest_command(test, command, deadline)
    rows = [test.read(MAILBOX+64+i*4) for i in range(8)]
    packet = b''.join(test.read(MAILBOX+128+i*4).to_bytes(4, 'little') for i in range(5))
    version = test.read(MAILBOX+160).to_bytes(4, 'little')
    if packet[12:] != b'\xff'*8 or version != b'\x06\xf9\xff\xff':
        raise ValueError('touch long-read bounds/version observations differ')
    return {'matrix': rows, 'touch_hex': packet.hex(), 'version_hex': version.hex()}


def validate_touch(observation, status, x, y):
    packet = bytes.fromhex(observation['touch_hex'])
    if packet[0] != status or ((packet[1] << 4) | (packet[3] >> 4)) != x or ((packet[2] << 4) | (packet[3] & 15)) != y:
        raise ValueError('guest touch packet status/coordinates differ')


def gdb_capture(gdb, output, sockets, initial):
    name = 'entry' if initial else 'terminal'
    commands = ['set pagination off', 'set confirm off', 'set auto-load off', 'set architecture arm',
                'file '+json.dumps(str(output/'guest/display.elf')),
                'target remote '+str(sockets/'gdb.sock'), 'python', 'import gdb,json', 'o={}']
    if initial:
        commands += ["o['initial_pc']=int(gdb.parse_and_eval('$pc'))",
                     "o['initial_mailbox']=int(gdb.parse_and_eval('*(unsigned int*)0x30001000'))",
                     "gdb.execute('stepi')", "o['stepped_pc']=int(gdb.parse_and_eval('$pc'))"]
    else:
        commands += ["gdb.Breakpoint('display_halt',type=gdb.BP_HARDWARE_BREAKPOINT)",
                     "gdb.execute('continue')", "o['stopped_pc']=int(gdb.parse_and_eval('$pc'))",
                     "o['halt_address']=int(gdb.parse_and_eval('&display_halt'))",
                     "o['mailbox']=[int(gdb.parse_and_eval('*(unsigned int*)%d'%(0x30001000+i*4))) for i in range(64)]",
                     "o['registers']={'r%d'%i:int(gdb.parse_and_eval('$r%d'%i)) & 0xffffffff for i in range(13)}",
                     "o['cpsr']=int(gdb.parse_and_eval('$cpsr'))"]
    commands += [f"with open({str(output/(name+'.json'))!r},'x') as f: json.dump(o,f,indent=2)", 'end', 'detach', 'quit']
    (output/(name+'.gdb')).write_text('\n'.join(commands)+'\n')
    with (output/(name+'-gdb.log')).open('xb') as log:
        subprocess.run([gdb, '--nx', '--nh', '--batch', '-x', str(output/(name+'.gdb'))], stdout=log,
                       stderr=subprocess.STDOUT, timeout=20, check=True)
    return json.loads((output/(name+'.json')).read_text())


def run(root, output, case, timeout=60, podman='podman', gdb='gdb'):
    if case not in CASES or not 1 <= timeout <= 120:
        raise ValueError('unsupported display/input case or timeout')
    root, output = root.resolve(), output.resolve()
    meta = checked_manifest(root)
    output.mkdir(parents=True, exist_ok=False)
    image_id = subprocess.check_output([podman, 'image', 'inspect', meta['image'], '--format', '{{.Id}}'], text=True, timeout=10).strip()
    if image_id != meta['image_id']:
        raise ValueError('runtime image identity changed')
    guest = build_guest(output/'guest')
    sockets = Path(tempfile.mkdtemp(prefix='prime-display-', dir=Path.home()/'.cache'))
    cid = output/'container.cid'
    args = [podman, 'run', '--rm', '--name', sockets.name, '--cidfile', str(cid), '--network', 'none',
            '--memory', '1g', '--ulimit', 'fsize=8388608:8388608', '--userns', 'keep-id', '--user', f'{os.getuid()}:{os.getgid()}',
            '--read-only', '--tmpfs', '/tmp:rw,size=32m', '-v', f'{root/"build"}:/build:ro,z',
            '-v', f'{output}:/run:rw,Z', '-v', f'{sockets}:/sockets:rw,Z', meta['image'],
            '/build/arm-softmmu/qemu-system-arm', '-M', 'hp-prime', '-kernel', '/run/guest/display.elf',
            '-display', 'none', '-monitor', 'none', '-serial', 'file:/run/uart.log', '-S', '-net', 'none',
            '-gdb', 'unix:/sockets/gdb.sock,server,nowait', '-qtest', 'unix:/sockets/qtest.sock,server,nowait',
            '-qtest-log', '/run/qtest.log', '-qmp', 'unix:/sockets/qmp.sock,server,nowait',
            '-d', 'in_asm,guest_errors', '-D', '/run/execution.log']
    report = {'schema_version': 1, 'scope': 'full_system_synthetic_hardware', 'case': case, 'status': 'failed',
              'machine': 'hp-prime', 'cpu': 'arm926', 'reference_revision': PIN, 'binary_sha256': meta['binary_sha256'],
              'image_id': meta['image_id'], 'patches': meta.get('patches', []), 'guest': guest,
              'physical_hardware_verified': False, 'kernel_boot_verified': False, 'guest_cpu_execution_verified': False,
              'observations': {}, 'command': args}
    process = None
    records = []
    test = None
    try:
        with (output/'stdout').open('xb') as stdout, (output/'stderr').open('xb') as stderr:
            process = subprocess.Popen(args, stdout=stdout, stderr=stderr, start_new_session=True)
            wait_socket(sockets/'gdb.sock', process)
            entry = gdb_capture(gdb, output, sockets, True)
            report['observations'].update(entry)
            if entry != {'initial_pc': 0x30000000, 'initial_mailbox': 0, 'stepped_pc': 0x30000004}:
                raise ValueError('guest entry/first instruction differs')
            deadline = time.monotonic()+timeout
            with socket.socket(socket.AF_UNIX) as qs, socket.socket(socket.AF_UNIX) as ms:
                qs.settimeout(5); ms.settimeout(5)
                qs.connect(str(sockets/'qtest.sock')); ms.connect(str(sockets/'qmp.sock'))
                with qs.makefile('rwb') as qstream, ms.makefile('rwb') as qmp:
                    records.append({'receive': json.loads(qmp.readline(65536))})
                    qmp_command(qmp, 'qmp_capabilities', records)
                    test = QTest(qstream)
                    wait_word(test, MAILBOX+12, 0x19d15a01, deadline)
                    if case == 'lcd':
                        qmp_command(qmp, 'screendump', records, {'filename': '/run/frame0.ppm'})
                        pixels = validate_ppm(output/'frame0.ppm')
                        write_png(output/'frame0.png', pixels)
                        guest_command(test, 2, deadline)
                        if test.read(MAILBOX+16) & 0x1800000 != 0x1800000:
                            raise ValueError('guest did not select/observe framebuffer1')
                        qmp_command(qmp, 'screendump', records, {'filename': '/run/frame1.ppm'})
                        pixels = validate_ppm(output/'frame1.ppm', True)
                        write_png(output/'frame1.png', pixels)
                        guest_command(test, 3, deadline)
                        if test.read(MAILBOX+20) != 0:
                            raise ValueError('guest display disable readback differs')
                        guest_command(test, 4, deadline)
                        qmp_command(qmp, 'screendump', records, {'filename': '/run/reenabled.ppm'})
                        validate_ppm(output/'reenabled.ppm', True)
                        report['observations']['frames'] = {n: digest(output/n) for n in ('frame0.ppm', 'frame1.ppm', 'reenabled.ppm')}
                        report['observations']['pixels_checked'] = 320*240*3
                    else:
                        samples = {}
                        samples['idle'] = sample(test, 10, deadline)
                        if samples['idle']['matrix'] != [0]*8 or bytes.fromhex(samples['idle']['touch_hex']) != b'\xff'*20:
                            raise ValueError('guest input was not idle')
                        key(qmp, records, 'p', True)
                        samples['p_down'] = sample(test, 11, deadline)
                        key(qmp, records, 'ret', True)
                        samples['two_keys'] = sample(test, 12, deadline)
                        key(qmp, records, 'p', False)
                        samples['p_up'] = sample(test, 13, deadline)
                        key(qmp, records, 'ret', False)
                        samples['all_up'] = sample(test, 14, deadline)
                        expected = {'p_down': [2]+[0]*7, 'two_keys': [130]+[0]*7, 'p_up': [128]+[0]*7, 'all_up': [0]*8}
                        for name, value in expected.items():
                            if samples[name]['matrix'] != value:
                                raise ValueError('guest keyboard sequence differs: '+name)
                        key(qmp, records, 'ret', True)
                        key(qmp, records, 'kp_enter', True)
                        samples['aliases_down'] = sample(test, 21, deadline)
                        key(qmp, records, 'ret', False)
                        samples['alias_still_down'] = sample(test, 22, deadline)
                        key(qmp, records, 'kp_enter', False)
                        samples['aliases_up'] = sample(test, 23, deadline)
                        if (samples['aliases_down']['matrix'] != [128]+[0]*7
                                or samples['alias_still_down']['matrix'] != [128]+[0]*7
                                or samples['aliases_up']['matrix'] != [0]*8):
                            raise ValueError('releasing one key alias cleared a held alias')
                        touch(qmp, records, 16384, 8192, True)
                        samples['touch_down'] = sample(test, 15, deadline)
                        validate_touch(samples['touch_down'], 9, 225, 80)
                        touch(qmp, records, 24576, 16384, True)
                        samples['touch_move'] = sample(test, 16, deadline)
                        validate_touch(samples['touch_move'], 10, 337, 160)
                        touch(qmp, records, 24576, 16384, False)
                        samples['touch_up'] = sample(test, 17, deadline)
                        validate_touch(samples['touch_up'], 11, 337, 160)
                        touch(qmp, records, -50, 50000, True)
                        samples['touch_clamped'] = sample(test, 18, deadline)
                        validate_touch(samples['touch_clamped'], 9, 0, 319)
                        touch(qmp, records, 0, 0, False)
                        key(qmp, records, 'p', True)
                        boot_count = test.read(0x30001100)
                        reset_event(qmp, records)
                        wait_word(test, 0x30001100, boot_count+1, deadline)
                        wait_word(test, MAILBOX+12, 0x19d15a01, deadline)
                        samples['reset'] = sample(test, 19, deadline)
                        if samples['reset']['matrix'] != [0]*8 or bytes.fromhex(samples['reset']['touch_hex']) != b'\xff'*20:
                            raise ValueError('reset leaked held input or touch packet')
                        key(qmp, records, 'p', False)
                        # Explicit cancellation releases every injected transition.
                        touch(qmp, records, 0, 0, False)
                        samples['cancelled'] = sample(test, 20, deadline)
                        if samples['cancelled']['matrix'] != [0]*8:
                            raise ValueError('cancelled input remained pressed')
                        report['observations']['input_samples'] = samples
                    guest_command(test, 99, deadline)
                    wait_word(test, MAILBOX, 0x600d0019, deadline)
                    report['observations'].update(gdb_capture(gdb, output, sockets, False))
                    if report['observations']['stopped_pc'] != report['observations']['halt_address']:
                        raise ValueError('guest terminal breakpoint differs')
                    qmp_command(qmp, 'quit', records)
            if process.wait(timeout=10) != 0:
                raise RuntimeError('research model exited unsuccessfully')
        if (output/'uart.log').read_bytes() != b'DI':
            raise ValueError('guest UART completion differs')
        report['observations']['uart_hex'] = (output/'uart.log').read_bytes().hex()
        report['status'] = 'passed'
        report['guest']['executed'] = True
        report['guest_cpu_execution_verified'] = True
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report['error'] = str(error)
    finally:
        if cid.is_file():
            owned_id = cid.read_text().strip()
            if len(owned_id) == 64 and all(c in '0123456789abcdef' for c in owned_id):
                subprocess.run([podman, 'rm', '--force', owned_id], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10, check=False)
        if process is not None and process.poll() is None:
            process.kill(); process.wait(timeout=10)
        shutil.rmtree(sockets)
        (output/'qmp.json').write_text(json.dumps(records, indent=2)+'\n')
        if test:
            (output/'commands.json').write_text(json.dumps(test.records, indent=2)+'\n')
        for name in ('execution.log', 'uart.log', 'entry-gdb.log', 'terminal-gdb.log'):
            if (output/name).is_file():
                report[name+'_sha256'] = digest(output/name)
        (output/'result.json').write_text(json.dumps(report, indent=2)+'\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--case', choices=CASES, required=True)
    parser.add_argument('--timeout', type=int, default=60)
    args = parser.parse_args()
    try:
        report = run(args.build_root, args.output, args.case, args.timeout)
        print(json.dumps(report, indent=2))
        return 0 if report['status'] == 'passed' else 1
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
