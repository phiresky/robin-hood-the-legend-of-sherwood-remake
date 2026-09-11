#!/usr/bin/env python3
"""Bounded synthetic input, only through the namespace-confined X transport.

Usage: client_x11.py key Escape | click X Y | move X Y | xtest-click X Y [BUTTON] | close
Keyboard/click use SendEvent; the client's XI2 mouse path needs xtest-click.
"""
import sys
import time

from namespace_x11 import open_display


def send_input(arguments):
    from Xlib import X, XK, protocol
    from Xlib.ext import xtest

    connection = open_display()
    def failed(error, _request):
        raise RuntimeError(f"client X input failed: {error}")
    connection.set_error_handler(failed)
    try:
        action = arguments[0]
        root = connection.screen().root
        if action == "xtest-click":
            button = int(arguments[3]) if len(arguments) > 3 else 1
            xtest.fake_input(connection, X.MotionNotify, x=int(arguments[1]), y=int(arguments[2]))
            connection.sync()
            xtest.fake_input(connection, X.ButtonPress, button)
            connection.sync()
            time.sleep(0.2)
            xtest.fake_input(connection, X.ButtonRelease, button)
            connection.sync()
            return
        targets = [window for window in root.query_tree().children
                   if "robin" in (window.get_wm_name() or "").lower()]
        if not targets:
            raise RuntimeError("No Robin window found")
        if action == "close":
            # Preserve the existing close-first / diagnostic-input-last policy.
            target = targets[0]
            target.send_event(protocol.event.ClientMessage(
                window=target, client_type=connection.intern_atom("WM_PROTOCOLS"),
                data=(32, [connection.intern_atom("WM_DELETE_WINDOW"), X.CurrentTime, 0, 0, 0])))
            connection.sync()
            return
        target = targets[-1]
        print(f"window={target.id} title={target.get_wm_name()}")
        target.set_input_focus(X.RevertToPointerRoot, X.CurrentTime)
        if action == "move":
            target.warp_pointer(int(arguments[1]), int(arguments[2]))
        elif action in ("key", "click"):
            key = action == "key"
            detail = connection.keysym_to_keycode(XK.string_to_keysym(arguments[1])) if key else 1
            px, py = (640, 480) if key else (int(arguments[1]), int(arguments[2]))
            if not key:
                target.warp_pointer(px, py)
            events = ((protocol.event.KeyPress, X.KeyPressMask),
                      (protocol.event.KeyRelease, X.KeyReleaseMask)) if key else (
                      (protocol.event.ButtonPress, X.ButtonPressMask),
                      (protocol.event.ButtonRelease, X.ButtonReleaseMask))
            for event_type, mask in events:
                event = event_type(time=X.CurrentTime, root=root, window=target,
                                   child=X.NONE, root_x=px, root_y=py,
                                   event_x=px, event_y=py, state=0,
                                   detail=detail, same_screen=1)
                target.send_event(event, propagate=True, event_mask=mask)
                connection.sync()
                time.sleep(0.15)
        else:
            raise RuntimeError(f"Unknown action: {action}")
        connection.sync()
    finally:
        connection.close()


if __name__ == "__main__":
    send_input(sys.argv[1:])
