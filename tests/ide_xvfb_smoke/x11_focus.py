#!/usr/bin/env python3
"""Give the IDE window keyboard focus on a bare X server (Xvfb has no window
manager, so no window is ever focused and the IDE's automation, which only
advances while the window is focused, would wait forever). Re-asserts focus
once a second until the IDE's window is gone or the timeout ends.
Usage: x11_focus.py <title substring> <timeout seconds>. Standard library only."""
import ctypes
import ctypes.util
import sys
import time

title, timeout = sys.argv[1].encode(), float(sys.argv[2])
x11 = ctypes.cdll.LoadLibrary(ctypes.util.find_library("X11") or "libX11.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XDefaultRootWindow.restype = ctypes.c_ulong
x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x11.XQueryTree.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong),
                           ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)),
                           ctypes.POINTER(ctypes.c_uint)]
x11.XFetchName.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_char_p)]
x11.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
x11.XFlush.argtypes = [ctypes.c_void_p]
x11.XFree.argtypes = [ctypes.c_void_p]
x11.XSetErrorHandler.restype = ctypes.c_void_p
# A window can vanish between listing and focusing it: ignore X errors.
HANDLER = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)(lambda d, e: 0)
x11.XSetErrorHandler(HANDLER)
dpy = x11.XOpenDisplay(None)
if not dpy:
    sys.exit("cannot open display")


def find(window):
    name = ctypes.c_char_p()
    if x11.XFetchName(dpy, window, ctypes.byref(name)) and name.value:
        found = title in name.value
        x11.XFree(name)
        if found:
            return window
    root, parent = ctypes.c_ulong(), ctypes.c_ulong()
    children, count = ctypes.POINTER(ctypes.c_ulong)(), ctypes.c_uint()
    if not x11.XQueryTree(dpy, window, ctypes.byref(root), ctypes.byref(parent),
                          ctypes.byref(children), ctypes.byref(count)):
        return None
    try:
        for i in range(count.value):
            hit = find(children[i])
            if hit:
                return hit
    finally:
        if children:
            x11.XFree(children)
    return None


deadline, seen = time.time() + timeout, False
while time.time() < deadline:
    window = find(x11.XDefaultRootWindow(dpy))
    if window:
        seen = True
        x11.XSetInputFocus(dpy, window, 2, 0)  # RevertToParent, CurrentTime
        x11.XFlush(dpy)
    elif seen:
        break
    time.sleep(1)
