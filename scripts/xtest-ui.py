"""Real X11 input and rendered-page clipboard reads, with no WebKit inspector."""

import ctypes
import os
import subprocess
import time
from pathlib import Path


class UI:
    def __init__(self, evidence, display):
        os.environ["DISPLAY"] = display
        import gi

        gi.require_version("Gtk", "3.0")
        gi.require_version("Gdk", "3.0")
        from gi.repository import Gdk, GLib, Gtk

        self.clipboard = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
        self.context = GLib.MainContext.default()
        self.evidence = Path(evidence)
        self.x = ctypes.CDLL("libX11.so.6")
        self.xt = ctypes.CDLL("libXtst.so.6")
        self.x.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.x.XOpenDisplay.restype = ctypes.c_void_p
        self.d = self.x.XOpenDisplay(display.encode())
        assert self.d, "X display unavailable"
        for name, args, result in [
            ("XDefaultRootWindow", [ctypes.c_void_p], ctypes.c_ulong),
            (
                "XQueryTree",
                [
                    ctypes.c_void_p,
                    ctypes.c_ulong,
                    ctypes.POINTER(ctypes.c_ulong),
                    ctypes.POINTER(ctypes.c_ulong),
                    ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)),
                    ctypes.POINTER(ctypes.c_uint),
                ],
                ctypes.c_int,
            ),
            (
                "XFetchName",
                [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_char_p)],
                ctypes.c_int,
            ),
            ("XFree", [ctypes.c_void_p], ctypes.c_int),
            ("XRaiseWindow", [ctypes.c_void_p, ctypes.c_ulong], ctypes.c_int),
            (
                "XResizeWindow",
                [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_uint, ctypes.c_uint],
                ctypes.c_int,
            ),
            (
                "XSetInputFocus",
                [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong],
                ctypes.c_int,
            ),
            ("XFlush", [ctypes.c_void_p], ctypes.c_int),
            ("XKeysymToKeycode", [ctypes.c_void_p, ctypes.c_ulong], ctypes.c_uint),
            ("XStringToKeysym", [ctypes.c_char_p], ctypes.c_ulong),
        ]:
            fn = getattr(self.x, name)
            fn.argtypes, fn.restype = args, result
        self.xt.XTestFakeMotionEvent.argtypes = [
            ctypes.c_void_p,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.c_ulong,
        ]
        self.xt.XTestFakeButtonEvent.argtypes = [
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_int,
            ctypes.c_ulong,
        ]
        self.xt.XTestFakeKeyEvent.argtypes = [
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_int,
            ctypes.c_ulong,
        ]
        self.win = self.wait(self.find, "native window")
        self.x.XRaiseWindow(self.d, self.win)
        self.x.XSetInputFocus(self.d, self.win, 2, 0)
        time.sleep(1)
        self.x.XResizeWindow(self.d, self.win, 1280, 800)
        self.x.XRaiseWindow(self.d, self.win)
        self.x.XSetInputFocus(self.d, self.win, 2, 0)
        self.x.XFlush(self.d)
        time.sleep(0.3)

    def find(self):
        a, b, n = ctypes.c_ulong(), ctypes.c_ulong(), ctypes.c_uint()
        children = ctypes.POINTER(ctypes.c_ulong)()
        self.x.XQueryTree(
            self.d,
            self.x.XDefaultRootWindow(self.d),
            ctypes.byref(a),
            ctypes.byref(b),
            ctypes.byref(children),
            ctypes.byref(n),
        )
        found = None
        for i in range(n.value):
            name = ctypes.c_char_p()
            self.x.XFetchName(self.d, children[i], ctypes.byref(name))
            if name.value == b"Mem Inspector":
                found = children[i]
            if name:
                self.x.XFree(name)
        if children:
            self.x.XFree(children)
        return found

    @staticmethod
    def wait(predicate, label, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            result = predicate()
            if result:
                return result
            time.sleep(0.1)
        raise AssertionError("Timed out: " + label)

    def click(self, x, y):
        self.xt.XTestFakeMotionEvent(self.d, 0, x, y, 0)
        for down in (1, 0):
            self.xt.XTestFakeButtonEvent(self.d, 1, down, 0)
        self.x.XFlush(self.d)
        time.sleep(0.15)

    def key(self, name, down):
        code = self.x.XKeysymToKeycode(self.d, self.x.XStringToKeysym(name.encode()))
        assert code
        self.xt.XTestFakeKeyEvent(self.d, code, down, 0)
        self.x.XFlush(self.d)

    def activate(self, key):
        """Use the UI's HTML access keys through actual XTest keyboard input."""
        self.key("Alt_L", 1)
        self.key(key, 1)
        self.key(key, 0)
        self.key("Alt_L", 0)
        time.sleep(0.2)

    def text(self):
        self.click(20, 760)
        self.clipboard.set_text("pending UI clipboard", -1)
        self.key("Control_L", 1)
        for name in ("a", "c"):
            self.key(name, 1)
            self.key(name, 0)
            time.sleep(0.05)
        self.key("Control_L", 0)
        time.sleep(0.05)
        result = []
        self.clipboard.request_text(
            lambda _clipboard, text, _data: result.append(text or ""), None
        )
        deadline = time.monotonic() + 5
        while not result and time.monotonic() < deadline:
            while self.context.pending():
                self.context.iteration(False)
            time.sleep(0.01)
        if not result:
            raise AssertionError("Native UI clipboard timeout")
        text = result[0]
        # Escape would dismiss a native review dialog. Right clears selection
        # without changing the review/approval state.
        self.key("Right", 1)
        self.key("Right", 0)
        return text

    def capture(self, name, modal=False):
        # Click the observed dialog's blank left padding, never an approval
        # button, to clear clipboard selection without dismissing the dialog.
        self.click(390, 400) if modal else self.click(20, 30)
        self.key("Right", 1)
        self.key("Right", 0)
        time.sleep(0.2)
        subprocess.run(
            ["import", "-window", "root", str(self.evidence / name)], check=True
        )
