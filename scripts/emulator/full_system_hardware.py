#!/usr/bin/env python3
"""Execute self-authored ARM926 timer/IRQ/FIQ/UART/GPIO/MMU hardware controls.

Guest code configures peripherals and executes its own exception handlers.
No proprietary firmware, USB, real kernel or calculator PPL backend is involved.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile

from full_system_preflight import inventory, PIN
from full_system_run import checked_manifest, qmp_command, wait_socket, digest
import struct

CASES = {'timer_irq': 1, 'timer_fiq': 2, 'uart_gpio': 3, 'mmu': 4}
LIMIT = 30


def build_guest(output, case):
    if case not in CASES:
        raise ValueError('unsupported hardware payload')
    output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve().parents[2] / 'research/full-system'
    tools = inventory(Path('/nonexistent'), [])['tools']
    compiler = tools['clang'] or tools['arm-none-eabi-gcc']
    linker = tools['ld.lld'] or tools['rust-lld']
    if not compiler or not linker:
        raise ValueError('hardware payload requires ARM compiler/linker')
    flags = ['--target=armv5-none-eabi'] if tools['clang'] else ['-mcpu=arm926ej-s']
    subprocess.run([compiler, *flags, '-DCASE=' + str(CASES[case]), '-g', '-c',
                    str(source/'hardware.S'), '-o', str(output/'hardware.o')], check=True, timeout=30)
    flavor = ['-flavor', 'gnu'] if linker == tools['rust-lld'] else []
    elf = output/'hardware.elf'
    subprocess.run([linker, *flavor, '-m', 'armelf', '-T', str(source/'hardware.ld'),
                    str(output/'hardware.o'), '-o', str(elf)], check=True, timeout=30)
    data = elf.read_bytes()
    if (data[:6] != bytes([127, 69, 76, 70, 1, 1]) or
            struct.unpack_from('<HH', data, 16) != (2, 40) or
            struct.unpack_from('<I', data, 24)[0] != 0x30000000):
        raise ValueError('unexpected ELF class/machine/entry')
    return {'path': str(elf), 'sha256': digest(elf), 'entry': '0x30000000',
            'source_sha256': digest(source/'hardware.S'), 'linked': True, 'executed': False}


def validate_observation(data, case, uart):
    if case not in CASES:
        raise ValueError('unsupported hardware observation')
    if data.get('initial_pc') != 0x30000000 or data.get('initial_mailbox') != 0:
        raise ValueError('guest was not initially fresh at its ELF entry')
    if data.get('stepped_pc') != 0x30000004:
        raise ValueError('first ARM instruction did not execute')
    if data.get('stopped_pc') != data.get('halt_address') or not data.get('halt_address'):
        raise ValueError('guest did not reach terminal breakpoint')
    words = data.get('mailbox', [])
    if not isinstance(words, list) or any(type(w) is not int or not 0 <= w <= 0xffffffff for w in words):
        raise ValueError('mailbox must contain bounded32bit integer words')
    if len(words) != 64 or words[0] != 0x600d0018 or words[4] != 0xfeed0018:
        raise ValueError('guest did not report successful post-handler mainline execution')
    if case in ('timer_irq', 'timer_fiq'):
        stack = 0x3000f000 if case == 'timer_irq' else 0x3000e000
        if (words[11] & 0x1f != 0x13 or not 0x30000000 <= words[12] < 0x30001000
                or not stack-256 <= words[13] < stack):
            raise ValueError('handler saved return state or banked stack differs')
    if case == 'timer_irq':
        if words[1:4] != [9, 0, 0] or words[5:7] != [5, 6]:
            raise ValueError('IRQ counts or masking observations differ')
        if words[17:26] != [10]*8+[11] or words[9] & 0x1f != 0x12:
            raise ValueError('IRQ source/order or actual handler mode differs')
        if uart != b'IIIIIIIIIE':
            raise ValueError('guest IRQ UART markers differ')
    elif case == 'timer_fiq':
        if words[1:4] != [0, 3, 0] or words[10] & 0x1f != 0x11:
            raise ValueError('FIQ count/isolation or handler mode differs')
        if uart != b'FFFE':
            raise ValueError('guest FIQ UART markers differ')
    elif case == 'uart_gpio':
        if uart != b'M18 UART\n' or words[7:9] != [0x5555, 0xa55a]:
            raise ValueError('guest UART or GPIO register readbacks differ')
    elif case == 'mmu':
        if words[7] != 0x12345678 or words[3] != 3:
            raise ValueError('MMU alias or abort count differs')
        if (words[8] & 0x1007 != 3 or words[11] != words[12] or
                words[14] != 1 or words[15] & 15 != 5):
            raise ValueError('MMU/alignment/cache state, repeated LDR or prefetch abort differs')
        faults = [(words[34] & 15, words[35]), (words[36] & 15, words[37]),
                  (words[38] & 15, words[39])]
        if faults != [(5, 0x20000000), (1, 0x30002001), (13, 0x10002000)]:
            raise ValueError('translation/alignment/permission fault observations differ: ' + repr(faults))
    data['uart_hex'] = uart.hex()
    return data


def run(build_root, output, case, timeout=LIMIT, podman='podman', gdb='gdb'):
    if case not in CASES or not 1 <= timeout <= 120:
        raise ValueError('unsupported case or deadline outside1..120seconds')
    build_root, output = build_root.resolve(), output.resolve()
    meta = checked_manifest(build_root)
    output.mkdir(parents=True, exist_ok=False)
    observed_image = subprocess.check_output(
        [podman, 'image', 'inspect', meta['image'], '--format', '{{.Id}}'], text=True, timeout=10).strip()
    if observed_image != meta['image_id']:
        raise ValueError('research runtime image differs from the build image')
    guest = build_guest(output / 'guest', case)
    cache = Path.home() / '.cache'
    sockets = Path(tempfile.mkdtemp(prefix='prime-gdb-', dir=cache))
    if len(str(sockets / 'gdb.sock').encode()) > 100:
        raise ValueError('debugger socket path exceeds portable Unix limit')
    cid = output / 'container.cid'
    arguments = [podman, 'run', '--rm', '--name', sockets.name, '--cidfile', str(cid),
                 '--network', 'none', '--memory', '1g', '--ulimit', 'fsize=8388608:8388608', '--userns', 'keep-id', '--user', f'{os.getuid()}:{os.getgid()}',
                 '--read-only', '--tmpfs', '/tmp:rw,size=32m',
                 '-v', f'{build_root / "build"}:/build:ro,z',
                 '-v', f'{output}:/run:rw,Z', '-v', f'{sockets}:/sockets:rw,Z',
                 meta['image'], '/build/arm-softmmu/qemu-system-arm',
                 '-M', 'hp-prime', '-kernel', '/run/guest/hardware.elf', '-display', 'none',
                 '-monitor', 'none', '-serial', 'file:/run/uart.log', '-S',
                 '-gdb', 'unix:/sockets/gdb.sock,server,nowait',
                 '-qmp', 'unix:/sockets/qmp.sock,server,nowait',
                 '-d', 'in_asm,guest_errors', '-D', '/run/execution.log', '-net', 'none', '-no-reboot']
    report = {'schema_version': 1, 'scope': 'full_system_synthetic_hardware', 'case': case, 'guest_cpu_execution_verified': False, 'physical_hardware_verified': False,
              'machine': 'hp-prime', 'cpu': 'arm926', 'kernel_boot_verified': False,
              'reference_revision': PIN, 'binary_sha256': meta['binary_sha256'],
              'image_id': meta['image_id'], 'patches': meta.get('patches', []), 'guest': guest, 'status': 'failed'}
    process = None
    qmp_records = []
    try:
        with (output / 'qemu.stdout').open('xb') as stdout, (output / 'qemu.stderr').open('xb') as stderr:
            process = subprocess.Popen(arguments, stdout=stdout, stderr=stderr, start_new_session=True)
            wait_socket(sockets / 'gdb.sock', process)
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
if obs['initial_pc'] != 0x30000000: raise gdb.GdbError('incorrect initial PC')
gdb.execute('stepi')
obs['stepped_pc']=int(gdb.parse_and_eval('$pc'))
gdb.Breakpoint('hardware_halt',type=gdb.BP_HARDWARE_BREAKPOINT)
gdb.execute('continue')
obs['stopped_pc']=int(gdb.parse_and_eval('$pc'))
obs['halt_address']=int(gdb.parse_and_eval('&hardware_halt'))
obs['mailbox']=[int(gdb.parse_and_eval('*(unsigned int*)%d'%(0x30001000+4*i))) for i in range(64)]
obs['registers']={{'r%d'%i:int(gdb.parse_and_eval('$r%d'%i)) & 0xffffffff for i in range(13)}}
obs['cpsr']=int(gdb.parse_and_eval('$cpsr'))
with open({result},'x') as stream: json.dump(obs,stream,indent=2)
end
quit
'''.format(elf=json.dumps(str(output / 'guest/hardware.elf')), socket=str(sockets / 'gdb.sock'),
           result=repr(str(output / 'observations.json')))
            (output / 'debug.gdb').write_text(script)
            with (output / 'gdb.log').open('xb') as log:
                subprocess.run([gdb, '--nx', '--nh', '--batch', '-x', str(output / 'debug.gdb')],
                               stdout=log, stderr=subprocess.STDOUT, timeout=timeout, check=True)
            report['observations'] = json.loads((output / 'observations.json').read_text())
            validate_observation(report['observations'], case, (output/'uart.log').read_bytes())
            with socket.socket(socket.AF_UNIX) as client:
                client.settimeout(5)
                client.connect(str(sockets / 'qmp.sock'))
                with client.makefile('rwb') as stream:
                    qmp_records.append({'receive': json.loads(stream.readline(65536))})
                    qmp_command(stream, 'qmp_capabilities', qmp_records)
                    qmp_command(stream, 'stop', qmp_records)
                    qmp_command(stream, 'quit', qmp_records)
            if process.wait(timeout=10) != 0:
                raise RuntimeError('research emulator exited unsuccessfully')
            report['guest']['executed'] = True
            report['guest_cpu_execution_verified'] = True
            report['status'] = 'passed'
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report['error'] = str(error)
        if process is not None:
            report['process_returncode_at_failure'] = process.poll()
    finally:
        # Killing Podman alone can orphan its container. Only our recorded ID is removed.
        if cid.is_file():
            owned_id = cid.read_text().strip()
            if len(owned_id) == 64 and all(c in '0123456789abcdef' for c in owned_id):
                subprocess.run([podman, 'rm', '--force', owned_id], stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, timeout=10, check=False)
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=10)
        shutil.rmtree(sockets)
        (output / 'qmp.json').write_text(json.dumps(qmp_records, indent=2) + '\n')
        for name in ['gdb.log', 'execution.log', 'uart.log']:
            path = output / name
            if path.is_file():
                report[name + '_sha256'] = digest(path)
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--case', choices=CASES, required=True)
    parser.add_argument('--timeout', type=int, default=LIMIT)
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
