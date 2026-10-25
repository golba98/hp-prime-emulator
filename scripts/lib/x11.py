"""Small ctypes bindings shared by the two offline X11 tools.

Libraries are loaded when a display is opened, never during import. Xlib owns
window-tree and title buffers; always release them with XFree, not Python's
allocator. No functions in this module communicate with a calculator.
"""

from __future__ import annotations

import ctypes
from collections.abc import Callable


class XMapRequest(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int),
        ("serial", ctypes.c_ulong),
        ("send_event", ctypes.c_int),
        ("display", ctypes.c_void_p),
        ("parent", ctypes.c_ulong),
        ("window", ctypes.c_ulong),
    ]


class XConfigureRequest(ctypes.Structure):
    _fields_ = [
        *XMapRequest._fields_,
        ("x", ctypes.c_int),
        ("y", ctypes.c_int),
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("border_width", ctypes.c_int),
        ("above", ctypes.c_ulong),
        ("detail", ctypes.c_int),
        ("value_mask", ctypes.c_ulong),
    ]


class XEvent(ctypes.Union):
    # Xlib defines XEvent's storage as 24 native longs on both ILP32 and LP64.
    _fields_ = [
        ("type", ctypes.c_int),
        ("map_request", XMapRequest),
        ("configure_request", XConfigureRequest),
        ("pad", ctypes.c_long * 24),
    ]


class XErrorEvent(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int),
        ("display", ctypes.c_void_p),
        ("resourceid", ctypes.c_ulong),
        ("serial", ctypes.c_ulong),
        ("error_code", ctypes.c_ubyte),
        ("request_code", ctypes.c_ubyte),
        ("minor_code", ctypes.c_ubyte),
    ]


ErrorHandler = ctypes.CFUNCTYPE(
    ctypes.c_int, ctypes.c_void_p, ctypes.POINTER(XErrorEvent)
)


def load_xlib() -> ctypes.CDLL:
    """Declare pointer-width-sensitive signatures before any native call."""
    library = ctypes.CDLL("libX11.so.6")
    pointer = ctypes.c_void_p
    window = ctypes.c_ulong
    integer = ctypes.c_int
    unsigned = ctypes.c_uint
    signatures = {
        "XOpenDisplay": ([ctypes.c_char_p], pointer),
        "XDefaultRootWindow": ([pointer], window),
        "XCloseDisplay": ([pointer], integer),
        "XQueryTree": (
            [
                pointer,
                window,
                ctypes.POINTER(window),
                ctypes.POINTER(window),
                ctypes.POINTER(ctypes.POINTER(window)),
                ctypes.POINTER(unsigned),
            ],
            integer,
        ),
        "XGetGeometry": (
            [
                pointer,
                window,
                ctypes.POINTER(window),
                ctypes.POINTER(integer),
                ctypes.POINTER(integer),
                ctypes.POINTER(unsigned),
                ctypes.POINTER(unsigned),
                ctypes.POINTER(unsigned),
                ctypes.POINTER(unsigned),
            ],
            integer,
        ),
        "XFetchName": (
            [pointer, window, ctypes.POINTER(ctypes.c_char_p)],
            integer,
        ),
        "XFree": ([pointer], integer),
        "XFlush": ([pointer], integer),
        "XSync": ([pointer, integer], integer),
        "XSetInputFocus": ([pointer, window, integer, window], integer),
        "XKeysymToKeycode": ([pointer, window], ctypes.c_ubyte),
        "XResizeWindow": ([pointer, window, unsigned, unsigned], integer),
        "XMoveResizeWindow": (
            [pointer, window, integer, integer, unsigned, unsigned],
            integer,
        ),
        "XMapWindow": ([pointer, window], integer),
        "XSelectInput": ([pointer, window, ctypes.c_long], integer),
        "XPending": ([pointer], integer),
        "XNextEvent": ([pointer, ctypes.POINTER(XEvent)], integer),
        "XSetErrorHandler": ([ErrorHandler], ctypes.c_void_p),
    }
    for name, (arguments, result) in signatures.items():
        function = getattr(library, name)
        function.argtypes = arguments
        function.restype = result
    return library


class Display:
    """An owned Xlib connection; use as a context manager."""

    def __init__(self, name: str):
        self.library = load_xlib()
        self.handle = self.library.XOpenDisplay(name.encode())
        if not self.handle:
            raise RuntimeError(f"cannot open display {name.encode()!r}")
        self.root = self.library.XDefaultRootWindow(self.handle)
        self._xtest = None
        self._error_handler = None
        self._previous_error_handler = None

    def __enter__(self) -> Display:
        return self

    def __exit__(self, *_exception: object) -> None:
        self.close()

    def close(self) -> None:
        if self.handle:
            self.library.XCloseDisplay(self.handle)
            self.handle = None
        if self._error_handler is not None:
            previous = ctypes.cast(self._previous_error_handler, ErrorHandler)
            self.library.XSetErrorHandler(previous)
            self._error_handler = None

    def handle_errors(self, callback: Callable[[int], None]) -> None:
        def handler(_display, event):
            callback(event.contents.error_code)
            return 0

        # Keep the callback alive until XCloseDisplay flushes pending work.
        self._error_handler = ErrorHandler(handler)
        self._previous_error_handler = self.library.XSetErrorHandler(
            self._error_handler
        )

    def windows(self) -> list[int]:
        root = ctypes.c_ulong()
        parent = ctypes.c_ulong()
        children = ctypes.POINTER(ctypes.c_ulong)()
        count = ctypes.c_uint()
        try:
            success = self.library.XQueryTree(
                self.handle,
                self.root,
                ctypes.byref(root),
                ctypes.byref(parent),
                ctypes.byref(children),
                ctypes.byref(count),
            )
            return (
                [children[index] for index in range(count.value)]
                if success
                else []
            )
        finally:
            if children:
                self.library.XFree(children)

    def title(self, window: int) -> str:
        title = ctypes.c_char_p()
        try:
            self.library.XFetchName(self.handle, window, ctypes.byref(title))
            return title.value.decode() if title.value else ""
        finally:
            if title:
                self.library.XFree(title)

    def geometry(self, window: int) -> tuple[int, int]:
        root = ctypes.c_ulong()
        x, y = ctypes.c_int(), ctypes.c_int()
        width, height = ctypes.c_uint(), ctypes.c_uint()
        border, depth = ctypes.c_uint(), ctypes.c_uint()
        success = self.library.XGetGeometry(
            self.handle,
            window,
            ctypes.byref(root),
            ctypes.byref(x),
            ctypes.byref(y),
            ctypes.byref(width),
            ctypes.byref(height),
            ctypes.byref(border),
            ctypes.byref(depth),
        )
        if not success:
            raise RuntimeError(f"cannot read geometry of window 0x{window:x}")
        return width.value, height.value

    def move_resize(self, window: int, width: int, height: int) -> None:
        self.library.XMoveResizeWindow(
            self.handle, window, 0, 0, width, height
        )

    def resize(self, window: int, width: int, height: int) -> None:
        self.library.XResizeWindow(self.handle, window, width, height)
        self.library.XFlush(self.handle)

    def focus(self, window: int) -> None:
        self.library.XSetInputFocus(self.handle, window, 2, 0)
        self.library.XFlush(self.handle)

    def key(self, symbol: int) -> None:
        """Send one press/release to this X server, never through USB."""
        if self._xtest is None:
            self._xtest = ctypes.CDLL("libXtst.so.6")
            self._xtest.XTestFakeKeyEvent.argtypes = [
                ctypes.c_void_p,
                ctypes.c_uint,
                ctypes.c_int,
                ctypes.c_ulong,
            ]
            self._xtest.XTestFakeKeyEvent.restype = ctypes.c_int
        code = self.library.XKeysymToKeycode(self.handle, symbol)
        if not code:
            raise RuntimeError(f"no X11 keycode for keysym {symbol:#x}")
        self._xtest.XTestFakeKeyEvent(self.handle, code, 1, 0)
        self._xtest.XTestFakeKeyEvent(self.handle, code, 0, 0)
        self.library.XFlush(self.handle)
