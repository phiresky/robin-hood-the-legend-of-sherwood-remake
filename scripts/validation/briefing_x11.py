#!/usr/bin/env python3
"""Disposable all-window input worker, confined to the local X namespace."""
import json
import sys
import time

from namespace_x11 import open_display


def dismiss_briefings(display):
    """Synthetic Return on every real Robin window, not merely the last one."""
    from Xlib import X, XK, protocol

    connection = open_display(display)
    def failed(error, _request):
        raise RuntimeError(f"briefing X input failed: {error}")
    connection.set_error_handler(failed)
    try:
        root = connection.screen().root
        targets = [window for window in root.query_tree().children
                   if "robin" in (window.get_wm_name() or "").lower()]
        if not targets:
            raise RuntimeError("no real Robin windows for briefing dismissal")
        detail = connection.keysym_to_keycode(XK.string_to_keysym("Return"))
        for target in targets:
            target.set_input_focus(X.RevertToPointerRoot, X.CurrentTime)
            for event_type, mask in ((protocol.event.KeyPress, X.KeyPressMask),
                                     (protocol.event.KeyRelease, X.KeyReleaseMask)):
                event = event_type(time=X.CurrentTime, root=root, window=target,
                                   child=X.NONE, root_x=640, root_y=480,
                                   event_x=640, event_y=480, state=0,
                                   detail=detail, same_screen=1)
                target.send_event(event, propagate=True, event_mask=mask)
                connection.sync()
                time.sleep(0.15)
        return [window.id for window in targets]
    finally:
        connection.close()


if __name__ == "__main__":
    print(json.dumps(dismiss_briefings(sys.argv[1])), flush=True)
