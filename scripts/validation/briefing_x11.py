#!/usr/bin/env python3
"""Disposable all-window Xlib input worker: a fatal X error cannot kill its parent."""
import ctypes as c
import json
import sys
import time

def dismiss_briefings(display):
    """Synthetic Return on each real Robin window (not physical input).

    Based on the parallel native-client diagnostic's libX11 injector.
    """
    x = c.CDLL("libX11.so.6")
    ptr, window = c.c_void_p, c.c_ulong
    x.XOpenDisplay.argtypes, x.XOpenDisplay.restype = [c.c_char_p], ptr
    x.XDefaultRootWindow.argtypes, x.XDefaultRootWindow.restype = [ptr], window
    x.XQueryTree.argtypes = [ptr, window, c.POINTER(window), c.POINTER(window), c.POINTER(c.POINTER(window)), c.POINTER(c.c_uint)]
    x.XFetchName.argtypes = [ptr, window, c.POINTER(c.c_char_p)]
    x.XStringToKeysym.argtypes, x.XStringToKeysym.restype = [c.c_char_p], c.c_ulong
    x.XKeysymToKeycode.argtypes, x.XKeysymToKeycode.restype = [ptr, c.c_ulong], c.c_uint
    x.XSendEvent.argtypes = [ptr, window, c.c_int, c.c_long, ptr]
    x.XFlush.argtypes = [ptr]
    x.XSetInputFocus.argtypes = [ptr, window, c.c_int, c.c_ulong]
    x.XCloseDisplay.argtypes = [ptr]
    x.XFree.argtypes = [ptr]
    class InputEvent(c.Structure):
        _fields_ = [("type", c.c_int), ("serial", c.c_ulong), ("send_event", c.c_int),
                    ("display", ptr), ("window", window), ("root", window),
                    ("subwindow", window), ("time", c.c_ulong), ("x", c.c_int),
                    ("y", c.c_int), ("x_root", c.c_int), ("y_root", c.c_int),
                    ("state", c.c_uint), ("detail", c.c_uint), ("same_screen", c.c_int)]
    connection = x.XOpenDisplay(display.encode())
    if not connection:
        raise RuntimeError("cannot open diagnostic X display")
    targets = []
    try:
        root = x.XDefaultRootWindow(connection)
        children, count = c.POINTER(window)(), c.c_uint()
        parent, returned_root = window(), window()
        if not x.XQueryTree(connection, root, c.byref(returned_root), c.byref(parent), c.byref(children), c.byref(count)):
            raise RuntimeError("cannot query diagnostic X display")
        for index in range(count.value):
            name = c.c_char_p()
            if x.XFetchName(connection, children[index], c.byref(name)) and name.value:
                if "robin" in name.value.decode(errors="replace").lower():
                    targets.append(children[index])
                x.XFree(name)
        x.XFree(children)
        if not targets:
            raise RuntimeError("no real Robin windows for briefing dismissal")
        detail = x.XKeysymToKeycode(connection, x.XStringToKeysym(b"Return"))
        for target in targets:
            x.XSetInputFocus(connection, target, 1, 0)
            for kind, mask in ((2, 1), (3, 2)):
                event = InputEvent(kind, 0, 1, connection, target, root, 0, 0, 640, 480, 640, 480, 0, detail, 1)
                backing = (c.c_long * 24)()
                c.memmove(backing, c.byref(event), c.sizeof(event))
                if not x.XSendEvent(connection, target, 1, mask, backing):
                    raise RuntimeError("briefing XSendEvent failed")
                x.XFlush(connection)
                time.sleep(0.15)
    finally:
        x.XCloseDisplay(connection)
    return targets


if __name__ == "__main__":
    print(json.dumps(dismiss_briefings(sys.argv[1])), flush=True)
