#!/usr/bin/env python3
"""Replay the pinned official mode-1 fragment builder inside Unicorn/x86.

No Windows process, imported vendor function, USB handle, or device is used.
Only 4c5a34..4c5b4e runs, with bounded memcpy and virtual-write stubs. This
establishes software framing, never physical acceptance or negotiation support.
Requires Unicorn (optional research dependency) and the retained PE32 binary.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import struct
from pathlib import Path
from ck_inspect import PE
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_EBX, UC_X86_REG_EBP, UC_X86_REG_ESI, UC_X86_REG_ESP, UC_X86_REG_EIP

HASH = "a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb"
ENTRY, STOP, MEMCPY = 0x4c5a34, 0x4c5b4e, 0xe7931a
RAM = 0x2000000
WRITE = RAM + 0x100
CTX, IO, VTABLE, MESSAGE, PAYLOAD, BUFFER = [RAM + n for n in (0x1000,0x2000,0x3000,0x4000,0x10000,0x400000)]
STACK = RAM + 0x500000

def u32(machine: Uc, address: int) -> int:
    return int.from_bytes(machine.mem_read(address, 4), "little")

def put32(machine: Uc, address: int, value: int) -> None:
    machine.mem_write(address, struct.pack("<I", value))

class Oracle:
    def __init__(self, pe: PE):
        self.uc = Uc(UC_ARCH_X86, UC_MODE_32)
        end=max(s["va"]+max(s["virtual_size"],s["size"]) for s in pe.sections)
        self.uc.mem_map(pe.base,(end-pe.base+0xfff)&~0xfff)
        for s in pe.sections:
            if s["size"]: self.uc.mem_write(s["va"],pe.data[s["offset"]:s["offset"]+s["size"]])
        self.uc.mem_map(RAM,0x600000)
        put32(self.uc,0x1135868,0) # Vendor debug logging disabled.
        self.writes=[]
        self.uc.hook_add(UC_HOOK_CODE,self.hook)
    def hook(self,machine:Uc,address:int,_size:int,_data:object)->None:
        if address==STOP:
            machine.emu_stop()
        elif address==MEMCPY:
            stack=machine.reg_read(UC_X86_REG_ESP)
            dst,src,size=[u32(machine,stack+n) for n in (4,8,12)]
            if not (0<=size<=64 and PAYLOAD<=src<PAYLOAD+0x200000 and BUFFER<=dst<BUFFER+64):
                raise ValueError("unexpected vendor memcpy")
            machine.mem_write(dst,bytes(machine.mem_read(src,size)))
            machine.reg_write(UC_X86_REG_EAX,dst)
            machine.reg_write(UC_X86_REG_EIP,u32(machine,stack))
            machine.reg_write(UC_X86_REG_ESP,stack+4) # cdecl: caller cleans args.
        elif address==WRITE:
            stack=machine.reg_read(UC_X86_REG_ESP)
            data,size=u32(machine,stack+4),u32(machine,stack+8)
            if data!=BUFFER or size!=64: raise ValueError("unexpected virtual write")
            self.writes.append(bytes(machine.mem_read(data,size)))
            machine.reg_write(UC_X86_REG_EAX,1)
            machine.reg_write(UC_X86_REG_EIP,u32(machine,stack))
            machine.reg_write(UC_X86_REG_ESP,stack+12) # MSVC thiscall RET 8.
    def frames(self,payload:bytes,message_id:int)->list[bytes]:
        m=self.uc
        m.mem_write(CTX,b"\0"*0x100);m.mem_write(IO,b"\0"*0x100);m.mem_write(MESSAGE,b"\0"*0x100)
        put32(m,CTX+4,IO);put32(m,IO,VTABLE);put32(m,IO+4,64);put32(m,VTABLE+8,WRITE)
        put32(m,CTX+0x2c,MESSAGE);put32(m,CTX+0x24,message_id)
        put32(m,MESSAGE,len(payload));put32(m,MESSAGE+0x24,PAYLOAD)
        m.mem_write(PAYLOAD,payload)
        m.mem_write(BUFFER,b"\1"+b"\0"*63)
        put32(m,STACK-0x334,BUFFER)
        self.writes=[];canonical=[]
        while u32(m,CTX+0x20)<len(payload):
            old=u32(m,CTX+0x20)
            for register,value in [(UC_X86_REG_EBP,STACK),(UC_X86_REG_ESP,STACK-0x800),(UC_X86_REG_ESI,CTX)]:m.reg_write(register,value)
            m.emu_start(ENTRY,STOP+1,count=200)
            new=u32(m,CTX+0x20)
            if new<=old or len(self.writes)!=len(canonical)+1:raise ValueError("vendor builder made no progress")
            header=9 if old==0 else 1
            canonical.append(self.writes[-1][:header+new-old])
        return canonical

def main()->None:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary",type=Path)
    parser.add_argument("--out",type=Path,required=True)
    args=parser.parse_args()
    data=args.binary.read_bytes()
    if hashlib.sha256(data).hexdigest()!=HASH:raise ValueError("unreviewed vendor binary")
    pe=PE(args.binary)
    oracle=Oracle(pe)
    cases=[]
    for size in [1,55,56,63,16065,20076,70000]:
        payload=bytes((i*37+11)%256 for i in range(size));message_id=0x1234abcd
        frames=oracle.frames(payload,message_id)
        digest=hashlib.sha256(b"".join(len(f).to_bytes(4,"little")+f for f in frames)).hexdigest()
        cases.append({"size":size,"message_id":message_id,"reports":len(frames),"canonical_sha256":digest,"first":frames[0].hex(),"last":frames[-1].hex()})
    result={"binary_sha256":HASH,"executed_range":[hex(ENTRY),hex(STOP)],"hardware_access":False,"evidence":"official instructions emulated offline; not physical acceptance","cases":cases}
    with args.out.open("x") as out:json.dump(result,out,indent=2);out.write("\n")
    print(json.dumps(result,indent=2))
if __name__=="__main__":main()
