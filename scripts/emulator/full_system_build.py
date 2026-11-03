#!/usr/bin/env python3
"""Build pinned GPL qemuPrime in an owned rootless Podman environment.

Scratch reference and production HLE remain untouched. Downloads are restricted
by this script to an official open-source image/packages and pinned DTC source.
No proprietary firmware is an input. No images are published.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import uuid

from full_system_preflight import PIN

DTC_PIN = '558cd81bdd432769b59bff01240c44f82cfb1a9d'
DTC_URL = 'https://git.kernel.org/pub/scm/utils/dtc/dtc.git'
IMAGE = 'localhost/prime-m17-qemu-xenial'
FLAGS = ['--target-list=arm-softmmu', '--python=/usr/bin/python2.7', '--disable-werror',
         '--disable-sdl', '--disable-gtk', '--disable-vnc', '--disable-curses',
         '--disable-opengl', '--disable-tools', '--disable-docs', '--disable-kvm',
         '--disable-blobs', '--with-system-pixman']


def git(reference, *args):
    return subprocess.check_output(['git', '-C', str(reference), *args], text=True).strip()


def export(reference, revision, destination):
    destination.mkdir(parents=True, exist_ok=True)
    with subprocess.Popen(['git', '-C', str(reference), 'archive', revision], stdout=subprocess.PIPE) as archive:
        try:
            subprocess.run(['tar', '-xf', '-', '-C', str(destination)], stdin=archive.stdout,
                           check=True, timeout=120)
        finally:
            archive.stdout.close()
        if archive.wait(timeout=10) != 0:
            raise RuntimeError('pinned source archive failed')


def sha(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(65536), b''):
            value.update(block)
    return value.hexdigest()


def build(reference, output, image=IMAGE, dtc_reference=None, overlay=None, jobs=4):
    reference, output = reference.resolve(), output.resolve()
    if git(reference, 'rev-parse', 'HEAD') != PIN:
        raise ValueError('research reference is not the pinned qemuPrime revision')
    if not 1 <= jobs <= 8:
        raise ValueError('jobs must be within1..8')
    output.mkdir(parents=True, exist_ok=False)
    repo = Path(__file__).resolve().parents[2]
    containerfile = repo / 'research/full-system/Containerfile.xenial'
    record = {'schema_version': 1, 'status': 'building', 'reference_revision': PIN,
              'dtc_revision': DTC_PIN, 'image': image, 'configure_flags': FLAGS,
              'containerfile_sha256': sha(containerfile), 'patches': []}
    cid = output / 'build.cid'
    try:
        with (output / 'image-build.log').open('xb') as log:
            subprocess.run(['podman', 'build', '--force-rm', '--tag', image, '--file', str(containerfile),
                            str(containerfile.parent)], stdout=log, stderr=subprocess.STDOUT,
                           check=True, timeout=600)
        record['image_id'] = subprocess.check_output(
            ['podman', 'image', 'inspect', image, '--format', '{{.Id}}'], text=True).strip()
        export(reference, PIN, output / 'source')
        if dtc_reference is None:
            dtc_reference = output / 'dtc-reference'
            with (output / 'dtc-clone.log').open('xb') as log:
                subprocess.run(['git', 'clone', '--bare', DTC_URL, str(dtc_reference)],
                               stdout=log, stderr=subprocess.STDOUT, timeout=180, check=True)
        if git(dtc_reference, 'cat-file', '-t', DTC_PIN) != 'commit':
            raise ValueError('pinned DTC object is not available')
        export(dtc_reference, DTC_PIN, output / 'source/dtc')
        overlays = overlay if isinstance(overlay, (list, tuple)) else ([overlay] if overlay else [])
        for item in overlays:
            item = item.resolve()
            record['patches'].append({'name': item.name, 'sha256': sha(item)})
            with item.open('rb') as patch:
                subprocess.run(['patch', '--batch', '--forward', '-p1'], stdin=patch,
                               cwd=output / 'source', check=True, timeout=30)
        (output / 'build').mkdir()
        command = ['/src/configure', *FLAGS]
        # Fixed commands and flags only; no source path or user shell text is interpolated.
        shell = ' '.join(command) + ' > configure.log 2>&1; make -j' + str(jobs) + ' subdir-arm-softmmu > make.log 2>&1; cp /toolchain-packages.txt .; cc --version > compiler.txt'
        args = ['podman', 'run', '--rm', '--name', 'prime-build-' + uuid.uuid4().hex[:12],
                '--cidfile', str(cid), '--network', 'none', '--userns', 'keep-id',
                '--user', f'{os.getuid()}:{os.getgid()}', '--read-only',
                '--tmpfs', '/tmp:rw,size=256m', '-v', f'{output / "source"}:/src:ro,Z',
                '-v', f'{output / "build"}:/build:rw,Z', '-w', '/build', image, '/bin/sh', '-ec', shell]
        with (output / 'container-build.log').open('xb') as log:
            subprocess.run(args, stdout=log, stderr=subprocess.STDOUT, timeout=600, check=True)
        binary = output / 'build/arm-softmmu/qemu-system-arm'
        record.update(status='built', binary_sha256=sha(binary))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        record.update(status='failed', error=str(error))
    finally:
        if cid.is_file():
            identity = cid.read_text().strip()
            if len(identity) == 64 and all(c in '0123456789abcdef' for c in identity):
                subprocess.run(['podman', 'rm', '--force', identity], stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, timeout=10, check=False)
        (output / 'build.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--dtc-reference', type=Path)
    parser.add_argument('--overlay', type=Path, action='append', help='repeatable ordered GPL source overlay')
    parser.add_argument('--jobs', type=int, default=4)
    args = parser.parse_args()
    try:
        result = build(args.reference, args.output, dtc_reference=args.dtc_reference,
                       overlay=args.overlay, jobs=args.jobs)
        print(json.dumps(result, indent=2))
        return 0 if result['status'] == 'built' else 1
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
