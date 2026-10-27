#!/usr/bin/env python3
"""Reproduce pinned device-management disassembly. No vendor execution or USB.

Output must be new. Raw vendor binaries and captured personal data stay private.
Requires Capstone, like ck_inspect.py. Addresses are preferred virtual addresses.
"""
from __future__ import annotations
import argparse
import contextlib
import hashlib
import json
from pathlib import Path
from ck_inspect import PE

CK32 = 'a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb'
CK64 = '477e75bd1040f3387a1174d6a8c9be4ffb07cb0408bf4bd810142146420ab6d1'
ARM = 'ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39'
RANGES = {
    'inventory-ui': [(0x4315b0,0x43161f),(0x4503b0,0x4504d9)],
    'incoming-dispatch':[(0x44f110,0x44f248),(0x44f280,0x44f610)],
    'catalog-model':[(0x430ad5,0x430c95)],
    'settings-model':[(0x430020,0x430208),(0x4bf990,0x4bfa11)],
    'backup-export':[(0x5a1b20,0x5a1d75),(0x59ee70,0x59f4d0)],
    'export-routing':[(0x5a2290,0x5a2380)],
    'delete-builder':[(0x44fc60,0x44fcc3)],
    'device-dispatch':[(0x59e400,0x59e711)],
    'delete-state':[(0x59bc60,0x59c23c),(0x59fc50,0x59fde0)],
    'automatic-clock':[(0x42f5e0,0x42f694),(0x44ffc0,0x450048)],
    'format-negotiation':[(0x59e212,0x59e2b5),(0x4bb2b0,0x4bb2c3),(0x4c0030,0x4c0049)],
    'compressed-envelope':[(0x59f8e0,0x59f927),(0x59b930,0x59ba9e),(0x5a2d20,0x5a2d6c)],
}

def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main() -> None:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ck32',type=Path,required=True)
    parser.add_argument('--ck64',type=Path,required=True)
    parser.add_argument('--firmware',type=Path,required=True)
    parser.add_argument('--out',type=Path,required=True)
    args=parser.parse_args()
    for path,expected in [(args.ck32,CK32),(args.ck64,CK64),(args.firmware,ARM)]:
        if digest(path)!=expected: raise ValueError(f'unreviewed binary: {path}')
    args.out.mkdir(exist_ok=False)
    manifest={'hardware_access':False,'vendor_execution':False,'inputs':{},'files':{}}
    for name,path in [('ck32',args.ck32),('ck64',args.ck64),('firmware',args.firmware)]:
        manifest['inputs'][name]={'path':str(path),'sha256':digest(path),'bytes':path.stat().st_size}
    for name,path,ranges in [('ck32',args.ck32,RANGES),('ck64',args.ck64,{'delete-builder':[(0x1400589f0,0x140058ab0)],'file-builder-crc':[(0x1401e8f50,0x1401e909b),(0x1401ee980,0x1401ee9d5)],'automatic-clock':[(0x140034930,0x1400349b0),(0x140058ec0,0x140058f70)]})]:
        pe=PE(path)
        for label,spans in ranges.items():
            target=args.out/f'{name}-{label}.asm'
            with target.open('w') as output,contextlib.redirect_stdout(output):
                print('SHA256',digest(path),'base',hex(pe.base),'preferred VAs; disassembly is not a complete reachability proof')
                for start,end in spans:
                    print('RANGE',hex(start),hex(end),'RVA',hex(start-pe.base),'file',hex(pe.offset(start)))
                    pe.disasm(start,end)
            manifest['files'][target.name]={'sha256':digest(target),'ranges':spans}
    import capstone
    raw=args.firmware.read_bytes()
    # Pinned ELF LOAD: file 0x34, VA 0x30600000, file length 0x76f634.
    decoder=capstone.Cs(capstone.CS_ARCH_ARM,capstone.CS_MODE_ARM)
    for label,start,end in [('dispatch',0x30689250,0x30689630),('delete',0x3068adec,0x3068b100),('export',0x3068b884,0x3068c2f0),('inventory',0x3068fd40,0x3068ffcc)]:
        target=args.out/f'arm-{label}.asm'
        with target.open('w') as output:
            print('SHA256',ARM,'ELF LOAD file=0x34 VA=0x30600000; newer firmware, not build-15157 proof',file=output)
            for i in decoder.disasm(raw[start-0x30600000+0x34:end-0x30600000+0x34],start):
                print(f'{i.address:08x} {i.mnemonic} {i.op_str}',file=output)
        manifest['files'][target.name]={'sha256':digest(target),'range':[start,end],'file_offset':start-0x30600000+0x34}
    (args.out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
if __name__=='__main__': main()
