#!/usr/bin/env python3
"""Read-only inventory for the isolated qemuPrime research track; never boots firmware."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

PIN = '32e891a9fc568de3110343c3e35b9ec0d23f6f44'


def inventory(root, artifacts):
    tools = {name: shutil.which(name) for name in
             ('python2', 'python3', 'arm-none-eabi-gcc', 'clang', 'ld.lld',
              'qemu-system-arm', 'docker', 'podman')}
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
    if not tools['arm-none-eabi-gcc'] and not (tools['clang'] and tools['ld.lld']):
        blockers.append('bare_metal_compile_and_link_toolchain_missing')
    if not tools['qemu-system-arm']:
        blockers.append('qemu_binary_missing')
    return {'schema_version': 1, 'backend': 'full_system_research',
            'reference_revision': revision, 'required_revision': PIN,
            'tools': tools, 'artifacts': files, 'blockers': blockers,
            'kernel_boot_verified': False,
            'artifact_bootability_verified': False,
            'next_experiment': 'build pinned fork; boot self-authored ARM926 smoke ELF on hp-prime; inspect mailbox via GDB'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path, default=Path('scratch/research/qemuPrime'))
    parser.add_argument('--artifact', type=Path, action='append', default=[])
    args = parser.parse_args()
    report = inventory(args.reference, args.artifact)
    print(json.dumps(report, indent=2))
    return 1 if report['blockers'] else 0


if __name__ == '__main__':
    raise SystemExit(main())
