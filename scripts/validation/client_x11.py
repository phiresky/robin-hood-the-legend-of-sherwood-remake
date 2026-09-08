#!/usr/bin/env python3
"""Send bounded diagnostic input to the Robin window on an isolated X display.

Uses synthetic input, not physical hardware. Requires DISPLAY and libX11.
Usage: client_x11.py key Escape | click X Y | move X Y | xtest-click X Y [BUTTON] | close
XSendEvent keyboard events work in this client, but its XI2 mouse path needs
xtest-click (python-xlib, installed only in the temporary evidence directory).
"""
import ctypes as c
import sys
import time

from namespace_x11 import open_display

if sys.argv[1] in ("xtest-click", "close"):
    from Xlib import X, protocol
    from Xlib.ext import xtest
    connection = open_display()
    if sys.argv[1] == "close":
        for window in connection.screen().root.query_tree().children:
            if "robin" in (window.get_wm_name() or "").lower():
                window.send_event(protocol.event.ClientMessage(
                    window=window, client_type=connection.intern_atom("WM_PROTOCOLS"),
                    data=(32, [connection.intern_atom("WM_DELETE_WINDOW"), X.CurrentTime, 0, 0, 0])))
                connection.sync()
                connection.close()
                raise SystemExit(0)
        raise SystemExit("No Robin window found")
    button = int(sys.argv[4]) if len(sys.argv) > 4 else 1
    xtest.fake_input(connection, X.MotionNotify, x=int(sys.argv[2]), y=int(sys.argv[3]))
    connection.sync()
    xtest.fake_input(connection, X.ButtonPress, button)
    connection.sync()
    time.sleep(0.2)
    xtest.fake_input(connection, X.ButtonRelease, button)
    connection.sync()
    connection.close()
    raise SystemExit(0)

x = c.CDLL("libX11.so.6")
Display = c.c_void_p
Window = c.c_ulong
x.XOpenDisplay.argtypes = [c.c_char_p]
x.XOpenDisplay.restype = Display
x.XDefaultRootWindow.argtypes = [Display]
x.XDefaultRootWindow.restype = Window
x.XQueryTree.argtypes = [Display, Window, c.POINTER(Window), c.POINTER(Window), c.POINTER(c.POINTER(Window)), c.POINTER(c.c_uint)]
x.XFetchName.argtypes = [Display, Window, c.POINTER(c.c_char_p)]
x.XStringToKeysym.argtypes = [c.c_char_p]
x.XStringToKeysym.restype = c.c_ulong
x.XKeysymToKeycode.argtypes = [Display, c.c_ulong]
x.XKeysymToKeycode.restype = c.c_uint
x.XSendEvent.argtypes = [Display, Window, c.c_int, c.c_long, c.c_void_p]
x.XFlush.argtypes = [Display]
x.XSetInputFocus.argtypes = [Display, Window, c.c_int, c.c_ulong]
x.XWarpPointer.argtypes = [Display, Window, Window, c.c_int, c.c_int, c.c_uint, c.c_uint, c.c_int, c.c_int]
x.XCloseDisplay.argtypes = [Display]
x.XFree.argtypes = [c.c_void_p]

class InputEvent(c.Structure):
    _fields_ = [("type", c.c_int), ("serial", c.c_ulong), ("send_event", c.c_int),
                ("display", Display), ("window", Window), ("root", Window),
                ("subwindow", Window), ("time", c.c_ulong), ("x", c.c_int),
                ("y", c.c_int), ("x_root", c.c_int), ("y_root", c.c_int),
                ("state", c.c_uint), ("detail", c.c_uint), ("same_screen", c.c_int)]

d = x.XOpenDisplay(None)
if not d:
    raise SystemExit("Cannot open DISPLAY")
root = x.XDefaultRootWindow(d)
children = c.POINTER(Window)()
count = c.c_uint()
parent, returned_root = Window(), Window()
if not x.XQueryTree(d, root, c.byref(returned_root), c.byref(parent), c.byref(children), c.byref(count)):
    raise SystemExit("Cannot query display windows")
target = None
for i in range(count.value):
    name = c.c_char_p()
    if x.XFetchName(d, children[i], c.byref(name)) and name.value:
        title = name.value.decode(errors="replace")
        if "robin" in title.lower():
            target = children[i]
            print(f"window={target} title={title}")
        x.XFree(name)
x.XFree(children)
if target is None:
    raise SystemExit("No Robin window found")
x.XSetInputFocus(d, target, 1, 0)
action = sys.argv[1]
if action == "move":
    x.XWarpPointer(d, 0, target, 0, 0, 0, 0, int(sys.argv[2]), int(sys.argv[3]))
elif action in ("key", "click"):
    key = action == "key"
    detail = x.XKeysymToKeycode(d, x.XStringToKeysym(sys.argv[2].encode())) if key else 1
    px, py = (640, 480) if key else (int(sys.argv[2]), int(sys.argv[3]))
    if not key:
        x.XWarpPointer(d, 0, target, 0, 0, 0, 0, px, py)
    for event_type, mask in ((2, 1), (3, 2)) if key else ((4, 4), (5, 8)):
        event = InputEvent(event_type, 0, 1, d, target, root, 0, 0, px, py, px, py, 0, detail, 1)
        # XEvent is a 24-long union; keep its complete backing allocation valid.
        backing = (c.c_long * 24)()
        c.memmove(backing, c.byref(event), c.sizeof(event))
        if not x.XSendEvent(d, target, 1, mask, backing):
            raise SystemExit("XSendEvent failed")
        x.XFlush(d)
        time.sleep(0.15)
else:
    raise SystemExit(f"Unknown action: {action}")
x.XFlush(d)
x.XCloseDisplay(d)
