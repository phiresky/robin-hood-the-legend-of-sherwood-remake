"""Namespace-confined python-xlib transport for single-threaded diagnostics.

Linux filesystem Unix sockets are shared across network namespaces. Xvfb's
PID-based display locks can also be reused across PID namespaces. python-xlib
prefers that filesystem socket, unlike libX11's abstract-socket-first path, so
ordinary Display(':N') can inject input into another test's game.
"""

import os
import socket
import sys
from unittest.mock import patch


def _namespace_socket(name, protocol, host, number):
    if sys.platform != "linux" or protocol not in (None, "unix") or host not in ("", "unix"):
        raise RuntimeError(f"diagnostic requires a namespace-local Linux X display: {name!r}")
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        connection.connect(f"\0/tmp/.X11-unix/X{number}")
        connection.set_inheritable(False)
        return connection
    except BaseException:
        connection.close()
        raise


def open_display(name=None):
    """Connect without a filesystem/TCP fallback; missing local X is an error.

    python-xlib has no public socket-injection argument. Override its transport
    factory only during construction in these single-threaded test processes;
    the established Display owns the socket afterward. Do not use this adapter
    in a multithreaded application or mutate the installed package.
    """
    from Xlib import display
    from Xlib.support import connect

    name = name if name is not None else os.environ["DISPLAY"]
    with patch.object(connect, "get_socket", _namespace_socket):
        return display.Display(name)
