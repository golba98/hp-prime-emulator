#!/usr/bin/env python3
"""Minimal X11 window manager for Xvfb sessions.

Enough to give override-less clients a real window size and stacking
order so that `import -window root` captures their pixels — e.g. taking
a genuine screenshot of the IDE under `tests/ide_xvfb_smoke`.

Usage: xvfb_miniwm.py <display> <width> <height> [duration_secs]

Not a general-purpose WM: every mapped window is placed at (0, 0) with
the requested size (WM_NORMAL_HINTS is honoured when present).
"""
import ctypes
import signal
import sys
import time

libX11 = ctypes.CDLL("libX11.so.6")

libX11.XOpenDisplay.restype = ctypes.c_void_p
libX11.XOpenDisplay.argtypes = [ctypes.c_char_p]
libX11.XMapWindow.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
libX11.XConfigureWindow.argtypes = [
    ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint, ctypes.c_void_p,
]
libX11.XSelectInput.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_long]
libX11.XNextEvent.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
libX11.XMoveResizeWindow.argtypes = [
    ctypes.c_void_p, ctypes.c_void_p,
    ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint,
]
libX11.XGetGeometry.argtypes = [
    ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong),
    ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int),
    ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint),
    ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint),
]

CWX, CWY, CWWidth, CWHeight, CWBorderWidth = 0x01, 0x02, 0x04, 0x08, 0x10
SubstructureRedirectMask = 1 << 20
SubstructureNotifyMask = 1 << 19
StructureNotifyMask = 1 << 17

MAP_REQUEST = 20
CONFIGURE_REQUEST = 23


def main():
    if len(sys.argv) < 4:
        print(__doc__)
        return 2
    display = sys.argv[1].encode()
    width, height = int(sys.argv[2]), int(sys.argv[3])
    duration = float(sys.argv[4]) if len(sys.argv) > 4 else 60.0

    d = libX11.XOpenDisplay(display)
    if not d:
        print(f"cannot open display {display!r}", flush=True)
        return 1
    ERROR_HANDLER = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)
    def on_error(disp, err):
        e = ctypes.cast(err, ctypes.POINTER(ctypes.c_ubyte)).contents
        print(f"wm: X error code={e.value}", flush=True)
        return 0
    handler = ERROR_HANDLER(on_error)
    libX11.XSetErrorHandler.argtypes = [ERROR_HANDLER]
    libX11.XSetErrorHandler(handler)
    libX11.XDefaultRootWindow.restype = ctypes.c_ulong
    root = libX11.XDefaultRootWindow(d)
    libX11.XSelectInput(d, ctypes.c_void_p(root), SubstructureRedirectMask | SubstructureNotifyMask)
    libX11.XSync.restype = ctypes.c_int
    libX11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
    libX11.XSync(d, False)
    print(f"wm: managing root {root} of {display!r}", flush=True)

    # XEvent is a union of structs; 192 bytes on 64-bit (XAnyEvent +
    # xclient data + pad). Too small a buffer would overflow.
    class XEvent(ctypes.Structure):
        _fields_ = [("type", ctypes.c_int), ("pad", ctypes.c_byte * 188)]

    class XMapRequest(ctypes.Structure):
        _fields_ = [
            ("type", ctypes.c_int), ("serial", ctypes.c_ulong),
            ("send_event", ctypes.c_int), ("display", ctypes.c_void_p),
            ("parent", ctypes.c_void_p), ("window", ctypes.c_void_p),
        ]

    class XConfigureRequest(ctypes.Structure):
        _fields_ = [
            ("type", ctypes.c_int), ("serial", ctypes.c_ulong),
            ("send_event", ctypes.c_int), ("display", ctypes.c_void_p),
            ("parent", ctypes.c_void_p), ("window", ctypes.c_void_p),
            ("x", ctypes.c_int), ("y", ctypes.c_int),
            ("width", ctypes.c_int), ("height", ctypes.c_int),
            ("border_width", ctypes.c_int), ("above", ctypes.c_void_p),
            ("detail", ctypes.c_int), ("value_mask", ctypes.c_ulong),
        ]

    def root_children():
        r = ctypes.c_ulong(); par = ctypes.c_ulong(); kids = ctypes.c_void_p(); n = ctypes.c_uint()
        libX11.XQueryTree(d, ctypes.c_void_p(root), ctypes.byref(r), ctypes.byref(par),
                          ctypes.byref(kids), ctypes.byref(n))
        if n.value == 0:
            return []
        arr = ctypes.cast(kids, ctypes.POINTER(ctypes.c_ulong))
        return [arr[i] for i in range(n.value)]

    def force_size():
        """Directly resize any root child that is smaller than the screen.

        winit creates override-redirect windows that never receive a
        MapRequest, so the MapRequest path alone leaves them at 1x1."""
        for wid in root_children():
            r = ctypes.c_ulong(); rx = ctypes.c_int(); ry = ctypes.c_int()
            rw = ctypes.c_uint(); rh = ctypes.c_uint(); rb = ctypes.c_uint(); rd = ctypes.c_uint()
            libX11.XGetGeometry(d, ctypes.c_void_p(wid), ctypes.byref(r), ctypes.byref(rx),
                                ctypes.byref(ry), ctypes.byref(rw), ctypes.byref(rh),
                                ctypes.byref(rb), ctypes.byref(rd))
            if rw.value < width or rh.value < height:
                libX11.XMoveResizeWindow(d, ctypes.c_void_p(wid), 0, 0, width, height)
                print(f"forced window 0x{wid:x} from {rw.value}x{rh.value} to {width}x{height}", flush=True)

    deadline = time.monotonic() + duration
    ev = XEvent()
    libX11.XPending.restype = ctypes.c_int
    libX11.XPending.argtypes = [ctypes.c_void_p]
    last_sweep = 0.0
    while time.monotonic() < deadline:
        now = time.monotonic()
        if now - last_sweep > 0.5:
            force_size()
            last_sweep = now
        if libX11.XPending(d) == 0:
            time.sleep(0.1)
            continue
        libX11.XNextEvent(d, ctypes.byref(ev))
        try:
            if ev.type == MAP_REQUEST:
                req = ctypes.cast(ctypes.byref(ev), ctypes.POINTER(XMapRequest)).contents
                w = int(req.window or 0)
                if w:
                    libX11.XMoveResizeWindow(d, ctypes.c_void_p(w), 0, 0, width, height)
                    libX11.XMapWindow(d, ctypes.c_void_p(w))
                    print(f"mapped window 0x{w:x} at {width}x{height}", flush=True)
            elif ev.type == CONFIGURE_REQUEST:
                req = ctypes.cast(ctypes.byref(ev), ctypes.POINTER(XConfigureRequest)).contents
                w = int(req.window or 0)
                cw = min(int(req.width) or width, 2 * width)
                ch = min(int(req.height) or height, 2 * height)
                if w:
                    libX11.XMoveResizeWindow(d, ctypes.c_void_p(w), 0, 0, cw, ch)
        except Exception as exc:  # a WM must never die on one bad window
            import traceback
            traceback.print_exc()
            print(f"wm: ignored {exc!r}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
