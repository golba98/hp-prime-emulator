#!/usr/bin/env python3
"""Probe real firmware Programs(name):=source through the existing runtime.
All profiles are temporary or beneath the supplied fresh artifacts directory.
This script changes no emulator implementation and never accesses USB.
"""
import argparse
import hashlib
import json
import os
import shutil
import struct
import subprocess
import tempfile
import time
from pathlib import Path


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    ap=argparse.ArgumentParser()
    ap.add_argument('--out',type=Path,required=True)
    args=ap.parse_args()
    out=args.out.resolve()
    out.mkdir(parents=True,exist_ok=False)
    runner=Path(os.environ['PRIME_FILE_COMPAT_RUNNER']).resolve()
    firmware=Path(os.environ['PRIME_TEST_FIRMWARE']).resolve()
    rom=Path(os.environ['PRIME_TEST_OS_ROM']).resolve()
    factory=Path(os.environ['PRIME_TEST_FACTORY_A']).resolve()
    initial=Path(os.environ['PRIME_TEST_INIT_DATA']).resolve()
    common=['--firmware',str(firmware),'--os-rom',str(rom),'--factory-a',str(factory),'--init-data',str(initial)]
    summary=dict(provenance='local-firmware',firmware_version='2.4.2',firmware_sha256=sha(firmware),runner_sha256=sha(runner))
    # Does Programs permit creation of a previously absent catalog object?
    # Updating the currently executing program is intentionally not attempted.
    source=out/'LIVEPROBE.ppl'
    source.write_text('EXPORT LIVEPROBE()\nBEGIN\n Programs("NEWLIVE"):="EXPORT NEWLIVE() BEGIN RETURN 3; END;";\n RETURN 1;\nEND;\n')
    command=[str(runner),'run-program',*common,'--program',str(source),'--out-dir',str(out/'create'),'--report',str(out/'create.json'),'--entry','LIVEPROBE','--execution-timeout-s','10','--max-seconds','45']
    with (out/'stdout.json').open('w') as stdout,(out/'stderr.log').open('w') as stderr:
        r=subprocess.run(command,stdout=stdout,stderr=stderr,timeout=90)
    report=json.loads((out/'create.json').read_text())
    summary['new_name_assignment']={k:report.get(k) for k in ['execution_status','compile_status','evidence','return_value','runtime_errors','work_dir']}
    summary['new_name_assignment']['exit_code']=r.returncode
    work=Path(report['work_dir']) if report.get('work_dir') else None
    files=list(work.rglob('NEWLIVE.hpprgm')) if work else []
    summary['new_name_assignment']['created_files']=[str(p.relative_to(out)) for p in files]
    # Separately, add a host file after a live session has booted, then ask
    # firmware to reopen its catalog. Compare its actual LCD before/after.
    with tempfile.TemporaryDirectory(prefix='hpprgm-live-') as tmp:
        data=Path(tmp)/'data'
        shutil.copytree(initial,data)
        subprocess.run([str(runner),'import','--program',str(source),'--data-dir',str(data)],check=True,capture_output=True)
        p=subprocess.Popen([str(runner),'session','--firmware',str(firmware),'--os-rom',str(rom),'--data-dir',str(data)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=(out/'session.log').open('wb'))
        import queue
        import threading
        packets=queue.Queue()
        def read_packets():
            try:
                while True:
                    h=p.stdout.read(5)
                    if len(h)!=5: break
                    n=struct.unpack('<I',h[:4])[0]
                    buf=bytearray()
                    while len(buf)<n:
                        b=p.stdout.read(n-len(buf))
                        if not b: return
                        buf.extend(b)
                    packets.put((h[4],bytes(buf)))
            finally: packets.put(None)
        threading.Thread(target=read_packets,daemon=True).start()
        def send(value):
            b=json.dumps(value).encode();p.stdin.write(struct.pack('<I',len(b))+b'\x03'+b);p.stdin.flush()
        def observe(seconds,label):
            frames=[];events=[];deadline=time.monotonic()+seconds
            while time.monotonic()<deadline:
                try: packet=packets.get(timeout=max(0.001,min(1,deadline-time.monotonic())))
                except queue.Empty: continue
                if packet is None: break
                kind,b=packet
                if kind==1: events.append(json.loads(b))
                elif kind==2: frames.append(b[16:])
            if frames:
                (out/(label+'.rgba')).write_bytes(frames[-1])
            return frames,events
        try:
            observe(5,'boot')
            send({'command':'script','keys':'SHIFT,1'})
            initial_frames,events=observe(5,'catalog-initial')
            send({'command':'script','keys':'HOME,ESC'})
            observe(3,'home')
            send({'command':'run','request':7,'source':source.read_text(),'entry':'LIVEPROBE','arguments':''})
            _,run_events=observe(30,'after-live-assignment')
            summary['same_session_assignment']={'events':run_events,
                'created_file':(data/'C/DATA/NEWLIVE.hpprgm').is_file()}
            # The existing managed Run command can fail catalog attribution.
            # Independently use normal firmware Editor Check and Home keys;
            # this tests creation without any guest-call or runtime changes.
            send({'command':'script','keys':'HOME,ESC,SHIFT,1,TAP:26:229,TAP:293:229,ENTER,HOME,ESC,ALPHA:LIVEPROBE,ENTER,HOME,ESC,SHIFT,1'})
            ui_frames,ui_events=observe(18,'catalog-ui-assignment')
            summary['ui_assignment']={'created_file':(data/'C/DATA/NEWLIVE.hpprgm').is_file(),'events':ui_events,
                'catalog_changed':bool(initial_frames and ui_frames and initial_frames[-1][320*20*4:320*220*4]!=ui_frames[-1][320*20*4:320*220*4])}
            send({'command':'script','keys':'HOME,ESC,SHIFT,1'})
            before,catalog_events=observe(5,'catalog-before')
            # Reuse an authored binary fixture. Host-file installation itself
            # is tested separately; this probes firmware's rescan behavior.
            fixture=Path(__file__).resolve().parent/'files/MINIMAL.hpprgm'
            shutil.copyfile(fixture,data/'C/DATA/MINIMAL.hpprgm')
            send({'command':'script','keys':'HOME,ESC,SHIFT,1'})
            after,more=observe(6,'catalog-after')
            summary['host_live_install']={'input_sha256':sha(fixture),'events':events+more,
                'before_sha256':hashlib.sha256(before[-1]).hexdigest() if before else None,
                'after_sha256':hashlib.sha256(after[-1]).hexdigest() if after else None,
                'identical_final_frame':bool(before and after and before[-1]==after[-1]),
                'identical_catalog_rows':bool(before and after and before[-1][320*20*4:320*220*4]==after[-1][320*20*4:320*220*4])}
        finally:
            if p.poll() is None: send({'command':'shutdown'})
            try: p.wait(timeout=10)
            except subprocess.TimeoutExpired:p.kill();p.wait()
        created=data/'C/DATA/NEWLIVE.hpprgm'
        if created.is_file():
            reopened=subprocess.run([str(runner),'run','--firmware',str(firmware),'--os-rom',str(rom),'--data-dir',str(data),
                '--on-unsupported','zero','--keys','SHIFT,1,DOWN,DOWN,TAP:26:229,ESC','--trace-fs','--storage-diff',
                '--max-instructions','8000000000','--max-seconds','45','--report',str(out/'reboot.json'),
                '--screenshot-dir',str(out/'reboot-shots')],capture_output=True,timeout=65)
            inspection=subprocess.run([str(runner),'inspect-program',str(created)],capture_output=True,text=True,check=True)
            (out/'reopened-source.txt').write_text(inspection.stdout)
            summary['ui_assignment']['reboot_exit_code']=reopened.returncode
            reboot=json.loads((out/'reboot.json').read_text())
            summary['ui_assignment']['reopened_in_editor']=any(e.get('op')=='wfopen' and e.get('step') is not None and e.get('mode','').startswith('w') and e.get('path','').lower().endswith('newlive.hpprgm') for e in reboot.get('fs_events',[]))
            summary['ui_assignment']['reopened_source_exact']=inspection.stdout.rsplit(' chars:\n',1)[-1]=='EXPORT NEWLIVE() BEGIN RETURN 3; END;\n'
            summary['ui_assignment']['saved_sha256']=sha(created)
    (out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print(json.dumps(summary,indent=2))


if __name__=='__main__':main()
