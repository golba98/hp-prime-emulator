#!/usr/bin/env python3
"""Host qtest LCD/input safety controls; no ARM execution claim."""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
from full_system_preflight import inventory, build_smoke
from full_system_run import checked_manifest, qmp_command, wait_socket
from full_system_intc import QTest, reset_event

LCD = 0x4c800000
I2C = 0x54000000
GPIO = 0x56000000


def check_model(test, qmp, records):
    # S3C2416/S3C2443 window1 OSD stride is12, not the borrowed Exynos16.
    test.write(LCD+0x28, 0)
    for off, value in ((0x34, 0x1002), (0x38, 0x3004), (0x3c, 0xabcdef)):
        test.write(LCD+off, value)
        if test.read(LCD+off) != value:
            raise ValueError('second-window OSD readback differs')
    if test.read(LCD+0x28) != 0:
        raise ValueError('second-window OSD write changed window0')
    test.write(LCD+0x80, 0x30200100)
    if test.read(LCD+0x80) != 0x30200100:
        raise ValueError('inactive framebuffer end address was discarded')
    test.write(LCD+0x14, 1 << 23)
    if test.read(LCD+0x14) & (1 << 24) == 0:
        raise ValueError('buffer select failed to update read-only status')
    test.write(LCD+0x14, 1 << 24)
    if test.read(LCD+0x14) & (1 << 24):
        raise ValueError('software wrote read-only buffer status')
    # Malformed/unmapped framebuffer addresses must not assert or map MMIO.
    test.write(LCD+0x10, (239 << 11) | 319)
    test.write(LCD+0x28, 0)
    test.write(LCD+0x2c, (319 << 11) | 239)
    test.write(LCD+0x64, 0x4a000000)
    test.write(LCD+0x94, 640)
    test.write(LCD+0x14, (5 << 2) | 1)
    test.write(LCD, 3)
    qmp_command(qmp, 'screendump', records, {'filename': '/run/invalid.ppm'})
    # Repeated resets exercise release of mapped framebuffer ownership.
    test.write(LCD, 0)
    test.write(LCD+0x64, 0x30010000)
    test.write(LCD, 3)
    qmp_command(qmp, 'screendump', records, {'filename': '/run/mapped.ppm'})
    for _ in range(3):
        reset_event(qmp, records)
        if test.read(LCD) != 0 or test.read(LCD+0x14) != 0:
            raise ValueError('LCD reset retained enable/window state')
        if test.read(GPIO+0x30) != 0 or test.read(GPIO+0x34) != 0 or test.read(GPIO+0x64) != 0:
            raise ValueError('GPIO reset retained matrix state')
    # Overlong address writes return NACK rather than negative-shift UB.
    test.write(I2C, 0xa0)
    test.write(I2C+4, 0xd0)
    test.write(I2C+12, 2)
    test.write(I2C+4, 0xf0)
    for data in (255, 0, 0, 99):
        test.write(I2C+12, data)
        test.write(I2C, 0xa0)
    if not test.read(I2C+4) & 1:
        raise ValueError('malformed touch address did not NACK')
    reset_event(qmp, records)
    return {'osd_window1_stride12': True, 'inactive_buffer_preserved': True,
            'buffer_status_readonly': True, 'invalid_framebuffer_safe': True,
            'mapped_reset_cycles': 3, 'gpio_reset': True, 'touch_address_nack': True}


def run(root, output):
    root, output = root.resolve(), output.resolve()
    meta = checked_manifest(root)
    output.mkdir(parents=True, exist_ok=False)
    image = subprocess.check_output(['podman', 'image', 'inspect', meta['image'], '--format', '{{.Id}}'], text=True, timeout=10).strip()
    if image != meta['image_id']:
        raise ValueError('runtime image identity changed')
    build_smoke(output/'guest', inventory(Path('/nonexistent'), [])['tools'])
    sockets = Path(tempfile.mkdtemp(prefix='prime-disp-model-', dir=Path.home()/'.cache'))
    cid = output/'container.cid'
    args = ['podman', 'run', '--rm', '--name', sockets.name, '--cidfile', str(cid), '--network', 'none',
            '--memory', '1g', '--ulimit', 'fsize=8388608:8388608', '--userns', 'keep-id', '--user', f'{os.getuid()}:{os.getgid()}',
            '--read-only', '--tmpfs', '/tmp:rw,size=32m', '-v', f'{root/"build"}:/build:ro,z', '-v', f'{output}:/run:rw,Z',
            '-v', f'{sockets}:/sockets:rw,Z', meta['image'], '/build/arm-softmmu/qemu-system-arm', '-M', 'hp-prime,accel=qtest',
            '-kernel', '/run/guest/smoke.elf', '-display', 'none', '-monitor', 'none', '-net', 'none',
            '-qtest', 'unix:/sockets/qtest.sock,server,nowait', '-qtest-log', '/run/qtest.log',
            '-qmp', 'unix:/sockets/qmp.sock,server,nowait', '-d', 'guest_errors,unimp', '-D', '/run/model.log']
    report = {'schema_version': 1, 'case': 'lcd_input_model', 'scope': 'qtest_s3c2416_device_model', 'status': 'failed',
              'guest_cpu_execution_verified': False, 'physical_hardware_verified': False,
              'kernel_boot_verified': False, 'binary_sha256': meta['binary_sha256'], 'image_id': meta['image_id'],
              'reference_revision': meta['reference_revision'], 'patches': meta.get('patches', [])}
    process = None
    records = []
    test = None
    try:
        with (output/'stdout').open('xb') as stdout, (output/'stderr').open('xb') as stderr:
            process = subprocess.Popen(args, stdout=stdout, stderr=stderr, start_new_session=True)
            wait_socket(sockets/'qtest.sock', process)
            with socket.socket(socket.AF_UNIX) as qs, socket.socket(socket.AF_UNIX) as ms:
                qs.settimeout(5); ms.settimeout(5)
                qs.connect(str(sockets/'qtest.sock')); ms.connect(str(sockets/'qmp.sock'))
                with qs.makefile('rwb') as qstream, ms.makefile('rwb') as qmp:
                    records.append({'receive': json.loads(qmp.readline(65536))})
                    qmp_command(qmp, 'qmp_capabilities', records)
                    test = QTest(qstream)
                    report['observations'] = check_model(test, qmp, records)
                    qmp_command(qmp, 'quit', records)
            if process.wait(timeout=10) != 0:
                raise RuntimeError('device-model process failed')
        report['status'] = 'passed'
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report['error'] = str(error)
    finally:
        if cid.is_file():
            owned_id = cid.read_text().strip()
            if len(owned_id) == 64 and all(c in '0123456789abcdef' for c in owned_id):
                subprocess.run(['podman', 'rm', '--force', owned_id], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10, check=False)
        if process is not None and process.poll() is None:
            process.kill(); process.wait(timeout=10)
        shutil.rmtree(sockets)
        (output/'qmp.json').write_text(json.dumps(records, indent=2)+'\n')
        if test:
            (output/'commands.json').write_text(json.dumps(test.records, indent=2)+'\n')
        (output/'result.json').write_text(json.dumps(report, indent=2)+'\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = run(args.build_root, args.output)
    print(json.dumps(report, indent=2))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
