#!/usr/bin/env python3
"""Real qtest MMIO/IRQ regressions for the isolated S3C2416 research model.

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
from full_system_preflight import build_smoke, inventory

INTC=0x4A000000
UART=0x50000000
BIT=0x10000000


class QTest:
    def __init__(self, stream):
        self.stream=stream
        self.records=[]
        self.events=[]

    def command(self, text):
        self.records.append({'send':text})
        self.stream.write((text+'\n').encode());self.stream.flush()
        for _ in range(64):
            line=self.stream.readline(4097)
            if not line or len(line)>4096:raise RuntimeError('qtest closed or response exceeded bound')
            response=line.decode().strip();self.records.append({'receive':response})
            if response.startswith('IRQ '):self.events.append(response);continue
            if not response.startswith('OK'):raise RuntimeError('qtest rejected command: '+response)
            return response
        raise RuntimeError('qtest event bound exceeded')

    def write(self, offset, value):self.command(f'writel {offset:#x} {value:#x}')
    def read(self, offset):return int(self.command(f'readl {offset:#x}').split()[1],0)
    def registers(self):
        return {name:self.read(INTC+offset) for name,offset in
                [('source',0),('mode',4),('pending',0x10),('offset',0x14),('subsource',0x18)]}


def reset_event(stream, records):
    start=len(records)
    qmp_command(stream,'system_reset',records)
    if any(r.get('receive',{}).get('event')=='RESET' for r in records[start:]):return
    for _ in range(64):
        line=stream.readline(65537)
        if not line or len(line)>65536:raise RuntimeError('QMP reset event missing')
        event=json.loads(line);records.append({'receive':event})
        if event.get('event')=='RESET':return
    raise RuntimeError('reset event bound exceeded')


def run_case(root, output, case):
    root,output=root.resolve(),output.resolve();meta=checked_manifest(root)
    output.mkdir(parents=True,exist_ok=False)
    image_id=subprocess.check_output(['podman','image','inspect',meta['image'],'--format','{{.Id}}'],text=True,timeout=10).strip()
    if image_id!=meta['image_id']:raise ValueError('runtime image changed')
    build_smoke(output/'guest',inventory(Path('/nonexistent'),[])['tools'])
    sockets=Path(tempfile.mkdtemp(prefix='prime-intc-',dir=Path.home()/'.cache'))
    cid=output/'container.cid'
    args=['podman','run','--rm','--name',sockets.name,'--cidfile',str(cid),'--network','none',
          '--memory','1g','--ulimit','fsize=8388608:8388608','--userns','keep-id','--user',f'{os.getuid()}:{os.getgid()}',
          '--read-only','--tmpfs','/tmp:rw,size=32m','-v',f'{root/"build"}:/build:ro,Z',
          '-v',f'{output}:/run:rw,Z','-v',f'{sockets}:/sockets:rw,Z',meta['image'],
          '/build/arm-softmmu/qemu-system-arm','-M','hp-prime,accel=qtest','-kernel','/run/guest/smoke.elf',
          '-display','none','-monitor','none','-serial','file:/run/uart.log','-S','-net','none',
          '-qtest','unix:/sockets/qtest.sock,server,nowait','-qtest-log','/run/qtest.log',
          '-qmp','unix:/sockets/qmp.sock,server,nowait','-d','guest_errors,unimp','-D','/run/model.log']
    result={'schema_version':1,'case':case,'scope':'qtest_s3c2416_device_model','status':'failed',
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
                    for cycle in range(3):
                        test.write(INTC+8,0xEFFFFFFF)
                        test.write(INTC+0x1C,0xFFFFFFFD)
                        if case=='fiq':test.write(INTC+4,BIT|0x400)
                        test.write(UART+0x20,0x41+cycle)
                        before=test.registers();result['observations'].append({'cycle':cycle,'before':before})
                        if before['source']!=BIT or before['subsource']!=2:raise RuntimeError('UART TX did not reach controller')
                        raised='IRQ raise '+('1' if case=='fiq' else '0')
                        if raised not in test.events:raise RuntimeError('expected CPU interrupt line was not raised')
                        if case=='fiq':
                            test.write(INTC+0x18,2);test.write(INTC,BIT);test.write(INTC+0x10,BIT)
                            after=test.registers();result['observations'][-1]['after_ack']=after
                            if after['source'] or after['subsource']:raise RuntimeError('FIQ acknowledgement retained pending state')
                            if 'IRQ lower 1' not in test.events:raise RuntimeError('FIQ acknowledgement did not lower CPU FIQ')
                        else:
                            reset_event(mstream,qmp_records)
                            after=test.registers();result['observations'][-1]['after_reset']=after
                            if any(after.values()):raise RuntimeError('reset retained pending/mode/offset state')
                            if 'IRQ lower 0' not in test.events:raise RuntimeError('reset did not lower CPU IRQ')
                        test.events.clear()
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
    parser.add_argument('--case',choices=['reset','fiq'],required=True)
    args=parser.parse_args()
    try:
        result=run_case(args.build_root,args.output,args.case);print(json.dumps(result,indent=2))
        return 0 if result['status']=='passed' else 1
    except (OSError,ValueError,subprocess.SubprocessError) as error:
        print(json.dumps({'status':'failed','error':str(error)}));return 2


if __name__=='__main__':raise SystemExit(main())
