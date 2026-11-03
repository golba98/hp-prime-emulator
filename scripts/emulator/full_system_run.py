#!/usr/bin/env python3
"""Execute only the self-authored ARM926 smoke on an isolated hp-prime QEMU.

No proprietary firmware or calculator access. This is a machine-model smoke,
not PRIME_OS boot and not a PPL FullSystem backend.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time

from full_system_preflight import build_smoke, inventory, PIN

LIMIT = 30
PATTERN = 0x12345678
ALIASES = [0x30002000, 0x32002000, 0x34002000, 0x36002000]


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(65536), b''):
            h.update(block)
    return h.hexdigest()


def validate_observation(data):
    if data.get('initial_pc') != 0x30000000:
        raise ValueError('CPU did not start at the smoke ELF entry')
    if data.get('initial_mailbox') != 0:
        raise ValueError('mailbox was not initially empty in a fresh process')
    if data.get('stepped_pc') != 0x30000004:
        raise ValueError('single-step did not execute the first ARM instruction')
    if data.get('stopped_pc') != data.get('halt_address') or not data.get('halt_address'):
        raise ValueError('guest did not halt at its declared breakpoint')
    if data.get('mailbox') != 0x600D0001:
        raise ValueError('guest reported failed or incomplete RAM alias test')
    if data.get('aliases') != [PATTERN] * 4:
        raise ValueError('RAM alias readbacks differ')
    return data


def checked_manifest(root):
    data = json.loads((root / 'build.json').read_text())
    if data.get('schema_version') != 1 or data.get('status') != 'built' or data.get('reference_revision') != PIN:
        raise ValueError('missing or incompatible pinned research build')
    binary = root / 'build/arm-softmmu/qemu-system-arm'
    if not binary.is_file() or digest(binary) != data.get('binary_sha256'):
        raise ValueError('research executable differs from its build manifest')
    return data


def qmp_command(stream, command, records, arguments=None):
    ident = len(records)
    request = {'execute': command, 'id': ident}
    if arguments is not None:
        request['arguments'] = arguments
    records.append({'send': request})
    stream.write((json.dumps(request) + '\n').encode())
    stream.flush()
    for _ in range(64):
        line = stream.readline(65537)
        if not line or len(line) > 65536:
            raise RuntimeError('invalid or closed QMP response')
        message = json.loads(line)
        records.append({'receive': message})
        if message.get('id') == ident:
            if 'error' in message:
                raise RuntimeError(str(message['error']))
            return message.get('return')
    raise RuntimeError('QMP response event bound exceeded')


def wait_socket(path, process):
    until = time.monotonic() + 10
    while time.monotonic() < until:
        if process.poll() is not None:
            raise RuntimeError('research container exited before opening its debugger')
        if path.exists():
            return
        time.sleep(0.02)
    raise TimeoutError('debugger socket did not appear')


def run(build_root, output, podman='podman', gdb='gdb'):
    build_root, output = build_root.resolve(), output.resolve()
    meta = checked_manifest(build_root)
    output.mkdir(parents=True, exist_ok=False)
    observed_image = subprocess.check_output(
        [podman, 'image', 'inspect', meta['image'], '--format', '{{.Id}}'], text=True, timeout=10).strip()
    if observed_image != meta['image_id']:
        raise ValueError('research runtime image differs from the build image')
    smoke = build_smoke(output / 'guest', inventory(Path('/nonexistent'), [])['tools'])
    cache = Path.home() / '.cache'
    sockets = Path(tempfile.mkdtemp(prefix='prime-gdb-', dir=cache))
    if len(str(sockets / 'gdb.sock').encode()) > 100:
        raise ValueError('debugger socket path exceeds portable Unix limit')
    cid = output / 'container.cid'
    arguments = [podman, 'run', '--rm', '--name', sockets.name, '--cidfile', str(cid),
                 '--network', 'none', '--memory', '1g', '--ulimit', 'fsize=8388608:8388608', '--userns', 'keep-id', '--user', f'{os.getuid()}:{os.getgid()}',
                 '--read-only', '--tmpfs', '/tmp:rw,size=32m',
                 '-v', f'{build_root / "build"}:/build:ro,Z',
                 '-v', f'{output}:/run:rw,Z', '-v', f'{sockets}:/sockets:rw,Z',
                 meta['image'], '/build/arm-softmmu/qemu-system-arm',
                 '-M', 'hp-prime', '-kernel', '/run/guest/smoke.elf', '-display', 'none',
                 '-monitor', 'none', '-serial', 'file:/run/uart.log', '-S',
                 '-gdb', 'unix:/sockets/gdb.sock,server,nowait',
                 '-qmp', 'unix:/sockets/qmp.sock,server,nowait',
                 '-d', 'in_asm,guest_errors', '-D', '/run/execution.log', '-net', 'none', '-no-reboot']
    report = {'schema_version': 1, 'scope': 's3c2416_machine_model_smoke',
              'machine': 'hp-prime', 'cpu': 'arm926', 'kernel_boot_verified': False,
              'reference_revision': PIN, 'binary_sha256': meta['binary_sha256'],
              'image_id': meta['image_id'], 'patches': meta.get('patches', []), 'smoke': smoke, 'status': 'failed'}
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
gdb.Breakpoint('smoke_halt',type=gdb.BP_HARDWARE_BREAKPOINT)
gdb.execute('continue')
obs['stopped_pc']=int(gdb.parse_and_eval('$pc'))
obs['halt_address']=int(gdb.parse_and_eval('&smoke_halt'))
obs['mailbox']=int(gdb.parse_and_eval('*(unsigned int*)0x30001000'))
obs['aliases']=[int(gdb.parse_and_eval('*(unsigned int*)%d'%a)) for a in {aliases}]
obs['registers']={{'r%d'%i:int(gdb.parse_and_eval('$r%d'%i)) for i in range(13)}}
obs['cpsr']=int(gdb.parse_and_eval('$cpsr'))
with open({result},'x') as stream: json.dump(obs,stream,indent=2)
end
quit
'''.format(elf=json.dumps(str(output / 'guest/smoke.elf')), socket=str(sockets / 'gdb.sock'),
           aliases=repr(ALIASES), result=repr(str(output / 'observations.json')))
            (output / 'debug.gdb').write_text(script)
            with (output / 'gdb.log').open('xb') as log:
                subprocess.run([gdb, '--nx', '--nh', '--batch', '-x', str(output / 'debug.gdb')],
                               stdout=log, stderr=subprocess.STDOUT, timeout=LIMIT, check=True)
            report['observations'] = validate_observation(json.loads((output / 'observations.json').read_text()))
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
            report['smoke']['executed'] = True
            report['status'] = 'passed'
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report['error'] = str(error)
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
    args = parser.parse_args()
    try:
        report = run(args.build_root, args.output)
        print(json.dumps(report, indent=2))
        return 0 if report['status'] == 'passed' else 1
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
