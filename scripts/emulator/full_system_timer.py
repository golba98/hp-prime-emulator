#!/usr/bin/env python3
"""Virtual-clock device regressions for the dedicated16-bit S3C2416 timer model.

Device-model execution only: guest CPU execution is established separately by
full_system_run.py. No physical hardware or proprietary firmware is accessed.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile

from full_system_run import checked_manifest, qmp_command, wait_socket
from full_system_intc import QTest, reset_event
from full_system_preflight import build_smoke, inventory

INTC=0x4A000000
UART=0x50000000
BIT=0x10000000


TIMER = 0x51000000


def check_channels(test, mstream, qmp_records, observations):
    for channel in range(5):
        bit = 1 << (10 + channel)
        shift = 0 if channel == 0 else 4 + channel * 4
        start, manual = 1 << shift, 2 << shift
        reload = (4 if channel == 4 else 8) << shift
        buffer = TIMER + 0xc + 12 * channel
        counter = TIMER + (0x40 if channel == 4 else 0x14 + 12 * channel)
        test.write(INTC + 8, 0xffffffff ^ bit)
        test.write(TIMER, 0)
        test.write(TIMER + 4, 0)
        test.write(buffer, 0x123456)
        if test.read(buffer) != 0x3456:
            raise RuntimeError('counter buffer did not enforce16bit width')
        test.write(buffer, 10)
        test.write(TIMER + 8, manual)
        if test.read(counter) != 10:
            raise RuntimeError('manual update did not load the buffered count')
        test.write(TIMER + 8, start)
        test.command('clock_step 1000')
        if test.read(INTC) != bit or test.read(INTC + 0x10) != bit:
            raise RuntimeError('oneshot expiry failed to latch correct source')
        if test.read(TIMER + 8) & start:
            raise RuntimeError('oneshot did not stop')
        test.write(INTC, bit)
        test.write(INTC + 0x10, bit)
        if test.read(INTC) or test.read(INTC + 0x10):
            raise RuntimeError('acknowledgement retained a request')
        test.command('clock_step 1000')
        if test.read(INTC):
            raise RuntimeError('oneshot retriggered')
        test.write(TIMER + 8, manual)
        test.write(TIMER + 8, start | reload)
        for _ in range(3):
            test.command('clock_step 1000')
            if test.read(INTC) != bit:
                raise RuntimeError('autoreload did not generate a new request')
            test.write(INTC, bit)
            test.write(INTC + 0x10, bit)
        test.write(TIMER + 8, 0)
        test.command('clock_step 10000')
        if test.read(INTC):
            raise RuntimeError('stopped channel leaked a callback')
        observations.append({'channel': channel, 'width16': True, 'manual':10,
                             'oneshot': True, 'autoreload_cycles': 3, 'stop': True})
        reset_event(mstream, qmp_records)
        if test.read(TIMER) or test.read(TIMER + 4) or test.read(TIMER + 8):
            raise RuntimeError('timer reset retained controls')
    # Internal divider and prescaler: 66MHz / 66 /16 =62500Hz, 10ticks160us.
    test.write(INTC + 8, 0xfffffbff)
    test.write(TIMER, 65)
    test.write(TIMER + 4, 3)
    test.write(TIMER + 0xc, 9)
    test.write(TIMER + 8, 2)
    test.write(TIMER + 8, 1)
    test.command('clock_step 150000')
    if test.read(INTC):
        raise RuntimeError('prescaler/divider expired too early')
    test.command('clock_step 20000')
    if test.read(INTC) != 0x400:
        raise RuntimeError('prescaler/divider did not expire')
    reset_event(mstream, qmp_records)
    # Explicitly unsupported external clock/DMA does not invent a clock.
    test.write(INTC + 8, 0xfffffbff)
    test.write(TIMER + 4, 4)
    test.write(TIMER + 0xc, 10)
    test.write(TIMER + 8, 2)
    test.write(TIMER + 8, 9)
    test.command('clock_step 1000000')
    if test.read(INTC):
        raise RuntimeError('unsupported external clock generated an interrupt')
    reset_event(mstream, qmp_records)
    test.write(TIMER + 0xc, 10)
    test.write(TIMER + 8, 2)
    test.write(TIMER + 8, 9)
    reset_event(mstream, qmp_records)
    test.command('clock_step 1000000')
    if test.read(INTC) or test.read(TIMER + 8):
        raise RuntimeError('reset leaked an active periodic callback')
    if test.read(TIMER + 0x44) != 0:
        raise RuntimeError('Exynos TINT_CSTAT was unexpectedly available')
    test.write(TIMER + 0x14, 0xffffffff)
    observations.append({'clock_formula': '66000000/66/16', 'external_clock_rejected':True,
                         'active_reset':True,'unsupported_registers_rejected':True})


def run_case(root, output):
    root,output=root.resolve(),output.resolve();meta=checked_manifest(root)
    output.mkdir(parents=True,exist_ok=False)
    image_id=subprocess.check_output(['podman','image','inspect',meta['image'],'--format','{{.Id}}'],text=True,timeout=10).strip()
    if image_id!=meta['image_id']:raise ValueError('runtime image changed')
    build_smoke(output/'guest',inventory(Path('/nonexistent'),[])['tools'])
    sockets=Path(tempfile.mkdtemp(prefix='prime-intc-',dir=Path.home()/'.cache'))
    cid=output/'container.cid'
    args=['podman','run','--rm','--name',sockets.name,'--cidfile',str(cid),'--network','none',
          '--memory','1g','--ulimit','fsize=8388608:8388608','--userns','keep-id','--user',f'{os.getuid()}:{os.getgid()}',
          '--read-only','--tmpfs','/tmp:rw,size=32m','-v',f'{root/"build"}:/build:ro,z',
          '-v',f'{output}:/run:rw,Z','-v',f'{sockets}:/sockets:rw,Z',meta['image'],
          '/build/arm-softmmu/qemu-system-arm','-M','hp-prime,accel=qtest','-kernel','/run/guest/smoke.elf',
          '-display','none','-monitor','none','-serial','file:/run/uart.log','-net','none',
          '-qtest','unix:/sockets/qtest.sock,server,nowait','-qtest-log','/run/qtest.log',
          '-qmp','unix:/sockets/qmp.sock,server,nowait','-d','guest_errors,unimp','-D','/run/model.log']
    result={'schema_version':1,'case':'timer_model','scope':'qtest_s3c2416_device_model','status':'failed',
            'physical_hardware_verified':False,'guest_cpu_execution_verified':False,
            'binary_sha256':meta['binary_sha256'],'image_id':meta['image_id'],
            'reference_revision':meta['reference_revision'],'patches':meta.get('patches',[]),'observations':[]}
    process=None;qmp_records=[];test=None
    try:
        with (output/'stdout').open('xb') as stdout,(output/'stderr').open('xb') as stderr:
            process=subprocess.Popen(args,stdout=stdout,stderr=stderr,start_new_session=True)
            wait_socket(sockets/'qtest.sock',process)
            with socket.socket(socket.AF_UNIX) as qsock,socket.socket(socket.AF_UNIX) as msock:
                qsock.settimeout(5);msock.settimeout(5)
                qsock.connect(str(sockets/'qtest.sock'));msock.connect(str(sockets/'qmp.sock'))
                with qsock.makefile('rwb') as qstream,msock.makefile('rwb') as mstream:
                    qmp_records.append({'receive':json.loads(mstream.readline(65536))})
                    qmp_command(mstream,'qmp_capabilities',qmp_records)
                    devices=qmp_command(mstream,'qom-list',qmp_records,{'path':'/machine/unattached'})
                    paths=['/machine/unattached/'+d['name'] for d in devices if d['type']=='child<arm926-arm-cpu>']
                    if len(paths)!=1:raise RuntimeError('cannot uniquely locate ARM926 CPU')
                    test=QTest(qstream);test.command('irq_intercept_in '+paths[0])
                    check_channels(test, mstream, qmp_records, result['observations'])
                    qmp_command(mstream,'quit',qmp_records)
            if process.wait(timeout=10)!=0:raise RuntimeError('device-model emulator exited unsuccessfully')
        result['status']='passed'
    except (OSError,ValueError,RuntimeError,subprocess.SubprocessError) as error:
        result['error']=str(error)
    finally:
        if cid.is_file():
            identity=cid.read_text().strip()
            if len(identity)==64 and all(c in '0123456789abcdef' for c in identity):
                subprocess.run(['podman','rm','--force',identity],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=10,check=False)
        if process is not None and process.poll() is None:process.kill();process.wait(timeout=10)
        shutil.rmtree(sockets)
        (output/'qmp.json').write_text(json.dumps(qmp_records,indent=2)+'\n')
        if test:(output/'commands.json').write_text(json.dumps(test.records,indent=2)+'\n')
        (output/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build-root',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    try:
        result=run_case(args.build_root,args.output);print(json.dumps(result,indent=2))
        return 0 if result['status']=='passed' else 1
    except (OSError,ValueError,subprocess.SubprocessError) as error:
        print(json.dumps({'status':'failed','error':str(error)}));return 2


if __name__=='__main__':raise SystemExit(main())
