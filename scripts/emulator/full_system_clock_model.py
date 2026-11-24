#!/usr/bin/env python3
"""Virtual-clock device regressions for the S3C2416 clock/reset subset.

Device-model execution only: guest CPU execution is established separately by
full_system_run.py. No physical hardware or proprietary firmware is accessed.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile

from full_system_run import checked_manifest, qmp_command, wait_socket, digest
from full_system_intc import QTest, reset_event
from full_system_preflight import build_smoke, inventory

INTC=0x4A000000
UART=0x50000000
BIT=0x10000000


TIMER = 0x51000000


def check_channels(test, mstream, qmp_records, observations):
    sysc = 0x4c000000
    def arm():
        test.write(TIMER + 8, 0)
        test.write(TIMER, 5)
        test.write(TIMER + 4, 0)
        test.write(TIMER + 0xc, 60000)
        test.write(TIMER + 8, 2)
        test.write(TIMER + 8, 1)
        return test.read(TIMER + 0x14)
    def elapsed(before, ns, expected):
        test.command('clock_step ' + str(ns))
        after = test.read(TIMER + 0x14)
        delta = before - after
        if abs(delta - expected) > 1:
            raise RuntimeError('clock count differs: ' + repr((before, after, delta, expected)))
        return after
    if test.read(sysc + 0x24) != 0xc or test.read(sysc + 0x70):
        raise RuntimeError('cold reset register state differs')
    test.write(sysc + 0x24, 0)
    count = arm()
    count = elapsed(count, 1000000, 1000)
    test.write(sysc + 0x24, 4)
    if abs(test.read(TIMER + 0x14) - count) > 1:
        raise RuntimeError('active divider update lost the counter')
    count = elapsed(count, 1000000, 500)
    gates = test.read(sysc + 0x34)
    test.write(sysc + 0x34, gates & ~0x400)
    count = elapsed(count, 1000000, 0)
    test.write(UART + 0x20, ord('A'))
    test.write(sysc + 0x34, gates & ~1)
    test.write(UART + 0x20, ord('B'))
    count = elapsed(count, 1000000, 500)
    test.write(sysc + 0x34, gates)
    test.write(UART + 0x20, ord('C'))
    # MPLL12MHz*200/(3*2)=400MHz, prediv2,HCLKdiv4,PCLKdiv2=25MHz.
    test.write(sysc + 0x10, (200 << 14) | (3 << 5) | 1)
    test.write(sysc + 0x24, 0x17)
    test.write(sysc + 0x20, 0x10)
    count = arm()
    elapsed(count, 120000, 250)
    # Reserved HCLK divider2 and zero PLL denominator stop safely.
    test.write(sysc + 0x24, 2)
    count = test.read(TIMER + 0x14)
    elapsed(count, 1000000, 0)
    test.write(sysc + 0x24, 0)
    test.write(sysc + 0x10, 0)
    count = test.read(TIMER + 0x14)
    elapsed(count, 1000000, 0)
    # UART SCLK selection/gating remains independent of PWM gating.
    test.write(sysc + 0x20, 0)
    test.write(UART + 4, 3 << 10)
    test.write(sysc + 0x38, test.read(sysc + 0x38) & ~0x100)
    test.write(UART + 0x20, ord('X'))
    test.write(sysc + 0x38, test.read(sysc + 0x38) | 0x100)
    test.write(UART + 0x20, ord('D'))
    test.write(sysc + 0x70, 0x19abcdef)
    test.write(sysc + 8, 0x1234)
    test.write(sysc + 0x44, 0x533c2443)
    if test.read(sysc + 0x70) != 0x19abcdef or test.read(sysc + 0x24) != 0:
        raise RuntimeError('invalid SWRST key caused reset')
    test.write(sysc + 0x44, 0x533c2416)
    for _ in range(64):
        event = json.loads(mstream.readline(65536))
        qmp_records.append({'receive': event})
        if event.get('event') == 'RESET':
            break
    else:
        raise RuntimeError('software reset event missing')
    if (test.read(sysc + 0x70) != 0x19abcdef or test.read(sysc + 8) != 0x1234 or
            test.read(sysc + 0x24) != 0xc or test.read(TIMER + 8) or test.read(sysc + 0x68) != 0x20):
        raise RuntimeError('software reset retention/clock/PWM state differs')
    reset_event(mstream, qmp_records)
    if test.read(sysc + 0x70) or test.read(sysc + 8) != 0x8000:
        raise RuntimeError('cold QMP reset did not restore defaults')
    count = arm()
    elapsed(count, 1000000, 500)
    observations.append({'profile': 's3c2416-tree', 'xti_hz': 12000000,
                         'clock_rates': True, 'active_divider_change': True,
                         'independent_gates': True, 'pll_path': True,
                         'reserved_clock_stopped': True, 'software_reset_retained': True,
                         'cold_reset_deterministic': True})


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
          '/build/arm-softmmu/qemu-system-arm','-M','hp-prime,accel=qtest','-global','s3c2416-sysc.clock-tree=on',
          '-global','s3c2416-sysc.xti-hz=12000000','-kernel','/run/guest/smoke.elf',
          '-display','none','-monitor','none','-serial','file:/run/uart.log','-net','none',
          '-qtest','unix:/sockets/qtest.sock,server,nowait','-qtest-log','/run/qtest.log',
          '-qmp','unix:/sockets/qmp.sock,server,nowait','-d','guest_errors,unimp','-D','/run/model.log']
    result={'schema_version':1,'case':'clock_model','scope':'qtest_s3c2416_device_model','status':'failed',
            'physical_hardware_verified':False,'guest_cpu_execution_verified':False,
            'binary_sha256':meta['binary_sha256'],'image_id':meta['image_id'],
            'reference_revision':meta['reference_revision'],'patches':meta.get('patches',[]),'observations':[],
            'clock_profile':{'mode':'s3c2416-tree','xti_hz':12000000},'command':args}
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
        if (output/'uart.log').read_bytes() != b'ACD':
            raise RuntimeError('UART clock-gate output differs')
        result['uart_hex']=(output/'uart.log').read_bytes().hex()
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
        for name in ['qtest.log','qmp.json','commands.json','model.log','uart.log']:
            if (output/name).is_file(): result[name+'_sha256']=digest(output/name)
        (output/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=['clock_model'], default='clock_model')
    parser.add_argument('--timeout', type=int, choices=range(1, 121), default=30)
    parser.add_argument('--build-root',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    def deadline(_signum, _frame):
        raise TimeoutError('clock-model deadline exceeded')
    signal.signal(signal.SIGALRM, deadline)
    signal.alarm(args.timeout)
    try:
        result=run_case(args.build_root,args.output);print(json.dumps(result,indent=2))
        return 0 if result['status']=='passed' else 1
    except (OSError,ValueError,subprocess.SubprocessError) as error:
        print(json.dumps({'status':'failed','error':str(error)}));return 2
    finally:
        signal.alarm(0)


if __name__=='__main__':raise SystemExit(main())
