#!/usr/bin/env python3
"""Read-only inventory for the isolated qemuPrime research track; never boots firmware."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import struct

PIN = '32e891a9fc568de3110343c3e35b9ec0d23f6f44'


def inventory(root, artifacts):
    tools = {name: shutil.which(name) for name in
             ('python2', 'python3', 'arm-none-eabi-gcc', 'clang', 'ld.lld',
              'qemu-system-arm', 'docker', 'podman')}
    tools['rust-lld'] = None
    rustc = shutil.which('rustc')
    if rustc:
        result = subprocess.run([rustc, '--print', 'sysroot'], capture_output=True, text=True, check=False)
        if result.returncode == 0:
            candidates = sorted(Path(result.stdout.strip()).glob('lib/rustlib/*/bin/rust-lld'))
            tools['rust-lld'] = str(candidates[0]) if candidates else None
    revision = None
    if root.is_dir():
        result = subprocess.run(['git', '-C', str(root), 'rev-parse', 'HEAD'],
                                capture_output=True, text=True, check=False)
        if result.returncode == 0:
            revision = result.stdout.strip()
    files = []
    for path in artifacts:
        if not path.is_file():
            files.append({'path': str(path), 'status': 'missing'})
            continue
        digest = hashlib.sha256()
        with path.open('rb') as stream:
            for block in iter(lambda: stream.read(65536), b''):
                digest.update(block)
        files.append({'path': str(path), 'status': 'inventoried_only',
                      'bytes': path.stat().st_size, 'sha256': digest.hexdigest()})
    blockers = []
    if revision != PIN:
        blockers.append('reference_checkout_missing_or_wrong_revision')
    if not tools['python2']:
        blockers.append('pinned_qemu_configure_requires_python2')
    if not tools['arm-none-eabi-gcc'] and not (tools['clang'] and (tools['ld.lld'] or tools['rust-lld'])):
        blockers.append('bare_metal_compile_and_link_toolchain_missing')
    if not tools['qemu-system-arm']:
        blockers.append('qemu_binary_missing')
    return {'schema_version': 1, 'backend': 'full_system_research',
            'reference_revision': revision, 'required_revision': PIN,
            'tools': tools, 'artifacts': files, 'blockers': blockers,
            'kernel_boot_verified': False,
            'artifact_bootability_verified': False,
            'next_experiment': 'build pinned fork; boot self-authored ARM926 smoke ELF on hp-prime; inspect mailbox via GDB'}


def build_smoke(output, tools):
    """Compile self-authored code into a NEW directory; never execute firmware."""
    output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve().parents[2] / 'research/full-system'
    compiler = tools['clang'] or tools['arm-none-eabi-gcc']
    linker = tools['ld.lld'] or tools['rust-lld']
    if not compiler or not linker:
        raise ValueError('smoke build requires Clang/GCC and ARM-capable LLD')
    obj, elf = output / 'smoke.o', output / 'smoke.elf'
    flags = ['--target=armv5-none-eabi'] if tools['clang'] else ['-mcpu=arm926ej-s']
    subprocess.run([compiler, *flags, '-c', str(source / 'smoke.S'), '-o', str(obj)], check=True)
    flavor = ['-flavor', 'gnu'] if linker == tools['rust-lld'] else []
    subprocess.run([linker, *flavor, '-m', 'armelf', '-T', str(source / 'smoke.ld'), str(obj), '-o', str(elf)], check=True)
    data = elf.read_bytes()
    if (data[:6] != bytes([127,69,76,70,1,1]) or
            struct.unpack_from('<HH', data, 16) != (2, 40) or
            struct.unpack_from('<I', data, 24)[0] != 0x30000000):
        raise ValueError('unexpected smoke ELF class, machine, type or entry')
    return {'path': str(elf), 'sha256': hashlib.sha256(data).hexdigest(),
            'entry': '0x30000000', 'linked': True, 'executed': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path, default=Path('scratch/research/qemuPrime'))
    parser.add_argument('--artifact', type=Path, action='append', default=[])
    parser.add_argument('--build-smoke', type=Path, help='compile/link smoke into a new directory; never run QEMU')
    args = parser.parse_args()
    report = inventory(args.reference, args.artifact)
    if args.build_smoke:
        try:
            report['smoke'] = build_smoke(args.build_smoke, report['tools'])
        except (OSError, ValueError, subprocess.CalledProcessError) as error:
            report['smoke_build_error'] = str(error)
            report['blockers'].append('smoke_build_failed')
    print(json.dumps(report, indent=2))
    return 1 if report['blockers'] else 0


if __name__ == '__main__':
    raise SystemExit(main())
