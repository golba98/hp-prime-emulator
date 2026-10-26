#!/usr/bin/env python3
"""Compare real native IDE processes on a private Xvfb display.

Requires cc, libX11, libXtst and Xvfb. Readback latency is XTest injection to
changed X11 pixels, including software-renderer submission and XGetImage
sampling; it is not compositor/monitor latency. Never opens USB devices.
Run through xvfb-run; no input is delivered to the user's desktop.
"""
from __future__ import annotations
import argparse
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import signal
import statistics
import subprocess
import sys
import tempfile
import time
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from scripts.lib.x11 import Display

class XImage(C.Structure):
    _fields_ = [("width",C.c_int),("height",C.c_int),("xoffset",C.c_int),("format",C.c_int),("data",C.c_void_p),("byte_order",C.c_int),("bitmap_unit",C.c_int),("bitmap_bit_order",C.c_int),("bitmap_pad",C.c_int),("depth",C.c_int),("bytes_per_line",C.c_int),("bits_per_pixel",C.c_int)]

def pixels(display: Display, window: int, x: int, y: int, width: int, height: int) -> bytes:
    lib=display.library
    lib.XGetImage.argtypes=[C.c_void_p,C.c_ulong,C.c_int,C.c_int,C.c_uint,C.c_uint,C.c_ulong,C.c_int]
    lib.XGetImage.restype=C.POINTER(XImage)
    lib.XDestroyImage.argtypes=[C.POINTER(XImage)]
    image=lib.XGetImage(display.handle,window,x,y,width,height,C.c_ulong(-1),2)
    if not image:
        return b""
    try:
        return C.string_at(image.contents.data,image.contents.bytes_per_line*height)
    finally:
        lib.XDestroyImage(image)

def wait_until(predicate, timeout=3.0):
    deadline=time.perf_counter()+timeout
    while time.perf_counter()<deadline:
        result=predicate()
        if result:return result
        time.sleep(.002)
    raise TimeoutError("GUI did not acknowledge input")

def cpu(pid):
    fields=Path(f"/proc/{pid}/stat").read_text().split()
    return (int(fields[13])+int(fields[14]))/os.sysconf("SC_CLK_TCK")

def rss(pid):
    return int(next(line.split()[1] for line in Path(f"/proc/{pid}/status").read_text().splitlines() if line.startswith("VmRSS:")))

def distribution(samples):
    ordered=sorted(samples)
    return {"n":len(samples),"p50_ms":round(statistics.median(samples),3),"p95_ms":round(ordered[min(len(ordered)-1,int(len(ordered)*.95))],3),"max_ms":round(max(samples),3)}

def trial(binary:Path, root:Path, library:Path, source:Path):
    root.mkdir(parents=True, exist_ok=True)
    env=os.environ.copy()
    for name in ["WAYLAND_DISPLAY","PPL_IDE_AUTOMATION","PPL_FIRMWARE","PPL_FACTORY_A"]:env.pop(name,None)
    env.update(XDG_CONFIG_HOME=str(root/"config"),XDG_DATA_HOME=str(root/"data"),XDG_CACHE_HOME=str(root/"cache"),LIBGL_ALWAYS_SOFTWARE="1",LP_NUM_THREADS="2",WINIT_UNIX_BACKEND="x11",LD_PRELOAD=str(library),PPL_ALLOC_COUNTER=str(root/"allocations.txt"))
    started=time.perf_counter()
    log=(root/"stderr.log").open("wb")
    app=subprocess.Popen([str(binary),"gui",str(source)],env=env,stdout=subprocess.DEVNULL,stderr=log)
    try:
        with Display(env["DISPLAY"]) as display:
            window=wait_until(lambda:next((w for w in display.windows() if "Native IDE" in display.title(w)),None),10)
            wait_until(lambda: len(set(pixels(display,window,0,0,160,22)))>5,10)
            startup=(time.perf_counter()-started)*1000
            display.move_resize(window,1920,1080);display.library.XFlush(display.handle)
            time.sleep(.5)
            display.focus(window)
            def allocations():
                file=root/"allocations.txt"
                before=len(file.read_text().splitlines()) if file.exists() else 0
                os.kill(app.pid,signal.SIGUSR1)
                wait_until(lambda:file.exists() and len(file.read_text().splitlines())>before)
                return tuple(map(int,file.read_text().splitlines()[-1].split()))
            a=allocations();t=time.perf_counter();c=cpu(app.pid)
            time.sleep(1.5)
            idle={"cpu_percent_one_core":round(100*(cpu(app.pid)-c)/(time.perf_counter()-t),3),"rss_kib":rss(app.pid)}
            b=allocations();idle.update(allocations=b[0]-a[0],requested_bytes=b[1]-a[1])
            xt=C.CDLL("libXtst.so.6")
            xt.XTestFakeMotionEvent.argtypes=[C.c_void_p,C.c_int,C.c_int,C.c_int,C.c_ulong]
            xt.XTestFakeButtonEvent.argtypes=[C.c_void_p,C.c_uint,C.c_int,C.c_ulong]
            xt.XTestFakeMotionEvent(display.handle,-1,600,180,0)
            xt.XTestFakeButtonEvent(display.handle,1,1,0);xt.XTestFakeButtonEvent(display.handle,1,0,0)
            display.library.XFlush(display.handle);time.sleep(.15)
            typed=[];a=allocations()
            for _ in range(20):
                before=pixels(display,window,0,1058,190,22);t=time.perf_counter();display.key(ord('x'))
                wait_until(lambda:pixels(display,window,0,1058,190,22)!=before)
                typed.append((time.perf_counter()-t)*1000)
            b=allocations();typing_alloc={"calls":b[0]-a[0],"requested_bytes":b[1]-a[1],"keypresses":20}
            scrolling=[]
            for _ in range(12):
                before=pixels(display,window,240,100,160,160);t=time.perf_counter()
                xt.XTestFakeButtonEvent(display.handle,5,1,0);xt.XTestFakeButtonEvent(display.handle,5,0,0);display.library.XFlush(display.handle)
                wait_until(lambda:pixels(display,window,240,100,160,160)!=before)
                scrolling.append((time.perf_counter()-t)*1000)
            resizing=[];a=allocations()
            for height in [1020,1080]*6:
                t=time.perf_counter();display.resize(window,1920,height)
                wait_until(lambda:len(set(pixels(display,window,0,height-22,190,22)))>5)
                resizing.append((time.perf_counter()-t)*1000)
                time.sleep(.03)
            b=allocations()
            return {"startup_first_nonblank_ms":round(startup,3),"idle":idle,"typing_readback":distribution(typed),"typing_allocations":typing_alloc,"scroll_readback":distribution(scrolling),"resize_readback":distribution(resizing),"resize_allocations":{"calls":b[0]-a[0],"requested_bytes":b[1]-a[1],"resizes":12}}
    finally:
        app.terminate()
        try:app.wait(timeout=5)
        except subprocess.TimeoutExpired:app.kill();app.wait()
        log.close()

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before",type=Path,required=True)
    parser.add_argument("--after",type=Path,required=True)
    parser.add_argument("--out",type=Path,required=True)
    parser.add_argument("--repeats",type=int,default=3)
    args=parser.parse_args()
    args.out.mkdir(parents=True,exist_ok=True)
    shared=Path(tempfile.gettempdir())/"malloc_counter.so"
    subprocess.run(["cc","-std=c11","-shared","-fPIC","-O2","-o",str(shared),str(Path(__file__).with_name("malloc_counter.c"))],check=True)
    source=args.out/"PERF.ppl"
    source.write_text("EXPORT PERF()\nBEGIN\n  LOCAL sum:=0;\n"+"".join(f"  // Row {i:04d}: scrolling and typing workload\n" for i in range(2000))+"  RETURN sum;\nEND;\n")
    report={"backend":"Private Xvfb, software GL, LP_NUM_THREADS=2","source_lines":2005,"latency_definition":"XTest input to changed XGetImage pixels; includes readback sampling, excludes monitor scanout","allocation_definition":"intercepted malloc/calloc/realloc calls and requested sizes; excludes aligned/GPU allocations","trials":{}}
    for label,binary in [("before",args.before),("after",args.after)]:
        binary=binary.resolve()
        results=[]
        for i in range(args.repeats):
            result=trial(binary,args.out/f"{label}-{i}",shared.resolve(),source.resolve())
            results.append(result);print(label,i,json.dumps(result),flush=True)
        report["trials"][label]={"binary_sha256":hashlib.sha256(binary.read_bytes()).hexdigest(),"samples":results}
    (args.out/"results.json").write_text(json.dumps(report,indent=2)+"\n")

if __name__=="__main__":main()
