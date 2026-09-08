#!/usr/bin/env python3
"""Bounded real X11 WM_DELETE_WINDOW checks on an already-isolated DISPLAY.

Usage: window_close.py BINARY EVIDENCE_DIR DATADIR MODE
MODE: startup, loading, briefing, gameplay, menu, startup-failure.
Requires ImageMagick and python-xlib (may use an isolated PYTHONPATH). The
harness never builds, changes existing saves, or sends input outside DISPLAY.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import urllib.request

from Xlib import X, XK, display, protocol
from Xlib.ext import xtest


def main():
    binary, destination, datadir, mode = sys.argv[1:]
    if mode not in {"startup", "loading", "briefing", "gameplay", "menu", "startup-failure"}:
        raise ValueError(f"unknown mode {mode}")
    destination = Path(destination).resolve()
    destination.mkdir(parents=True, exist_ok=False)
    environment = dict(os.environ)
    for name in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "ROBINHOOD_SAVE_DIR"):
        directory = destination / name.lower()
        directory.mkdir()
        environment[name] = str(directory)
    environment.update(
        ROBINHOOD_DATA_DIR=str(Path(datadir).resolve()),
        RUST_LOG="debug,wgpu_core=warn,wgpu_hal=warn,naga=warn,reqwest=warn,hyper=warn",
        RUST_BACKTRACE="1",
    )
    if mode == "startup-failure":
        environment["ROBINHOOD_DATA_DIR"] = str(destination / "missing-data")
    command = [str(Path(binary).resolve()), "--http-server", "17861"]
    if mode == "menu":
        command.append("--force-main-menu")
    log_path = destination / "client.log"
    start = time.monotonic()
    connection = display.Display(environment["DISPLAY"])
    result = {"mode": mode, "command": command, "display": environment["DISPLAY"],
              "datadir": environment["ROBINHOOD_DATA_DIR"], "close_sent": False}
    with log_path.open("w") as log:
        process = subprocess.Popen(command, cwd=destination, env=environment,
                                   stdout=log, stderr=subprocess.STDOUT)

        def wait_for(predicate, seconds=120):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(f"client exited before readiness: {process.returncode}")
                value = predicate()
                if value:
                    return value
                time.sleep(0.1)
            raise TimeoutError("readiness condition did not arrive")

        def find_window():
            for candidate in connection.screen().root.query_tree().children:
                if "robin" in (candidate.get_wm_name() or "").lower():
                    return candidate
            return None

        def frame():
            try:
                with urllib.request.urlopen("http://127.0.0.1:17861/host-debug", timeout=2) as response:
                    state = json.load(response)
                    samples = result.setdefault("frame_samples", [])
                    if not samples or samples[-1] != state.get("frame"):
                        samples.append(state.get("frame"))
                    return state.get("frame", 0)
            except (OSError, ValueError):
                return 0

        def key(window, name):
            window.set_input_focus(X.RevertToParent, X.CurrentTime)
            connection.sync()
            keycode = connection.keysym_to_keycode(XK.string_to_keysym(name))
            for kind, mask in ((X.KeyPress, X.KeyPressMask), (X.KeyRelease, X.KeyReleaseMask)):
                event_type = protocol.event.KeyPress if kind == X.KeyPress else protocol.event.KeyRelease
                window.send_event(event_type(time=X.CurrentTime, root=connection.screen().root,
                    window=window, child=X.NONE, root_x=0, root_y=0, event_x=0, event_y=0,
                    state=0, detail=keycode, same_screen=1), event_mask=mask)
                connection.sync()
                time.sleep(0.15)

        try:
            if mode == "startup-failure":
                result["returncode"] = process.wait(timeout=90)
                result["passed"] = result["returncode"] != 0
            else:
                window = wait_for(find_window)
                result["window_id"] = window.id
                if mode == "loading":
                    wait_for(lambda: "[loading]" in log_path.read_text())
                elif mode in {"briefing", "gameplay"}:
                    wait_for(lambda: "Autosave committed" in log_path.read_text())
                    wait_for(lambda: frame() >= 2)
                    if mode == "gameplay":
                        for _ in range(4):
                            key(window, "Return")
                            time.sleep(0.5)
                        wait_for(lambda: frame() > 10)
                elif mode == "menu":
                    wait_for(lambda: log_path.read_text().count("DEBUG fps:") >= 5)
                    time.sleep(3)
                    key(window, "Escape")
                    time.sleep(2)
                    result["escape_kept_process_alive"] = process.poll() is None
                    if not result["escape_kept_process_alive"]:
                        raise RuntimeError("Escape unexpectedly bypassed menu confirmation")
                    subprocess.run(["magick", "import", "-display", environment["DISPLAY"],
                                    "-window", str(window.id), str(destination / "after-escape.png")],
                                   check=True, timeout=10)
                    # The demo's fixed 640x480 menu is centered in 1024x768.
                    # Exercise its Quit button too; if Escape already opened
                    # confirmation, this outside-dialog click leaves it open.
                    xtest.fake_input(connection, X.MotionNotify, x=748, y=604)
                    xtest.fake_input(connection, X.ButtonPress, 1)
                    connection.sync()
                    time.sleep(0.2)
                    xtest.fake_input(connection, X.ButtonRelease, 1)
                    connection.sync()
                    time.sleep(2)
                    result["quit_button_kept_process_alive"] = process.poll() is None
                    if not result["quit_button_kept_process_alive"]:
                        raise RuntimeError("Quit button unexpectedly bypassed confirmation")
                # Preserve exact X pixels independently of game RPC. This is
                # raw evidence (typically BGRX), not an image-quality assertion.
                geometry = window.get_geometry()
                captured = window.get_image(0, 0, geometry.width, geometry.height, X.ZPixmap, 0xffffffff)
                (destination / "before-close.xpixels").write_bytes(captured.data)
                result["capture"] = {"width": geometry.width, "height": geometry.height,
                                     "depth": captured.depth, "bytes": len(captured.data)}
                subprocess.run(["magick", "import", "-display", environment["DISPLAY"],
                                "-window", str(window.id), str(destination / "before-close.png")],
                               check=True, timeout=10)
                result["close_at_s"] = round(time.monotonic() - start, 3)
                window.send_event(protocol.event.ClientMessage(window=window,
                    client_type=connection.intern_atom("WM_PROTOCOLS"),
                    data=(32, [connection.intern_atom("WM_DELETE_WINDOW"), X.CurrentTime, 0, 0, 0])))
                connection.sync()
                result["close_sent"] = True
                result["returncode"] = process.wait(timeout=90)
                result["shutdown_s"] = round(time.monotonic() - start - result["close_at_s"], 3)
                result["passed"] = result["returncode"] == 0
        except Exception as error:
            result.update(passed=False, error=repr(error))
        finally:
            if process.poll() is None:
                result["bounded_stop"] = True
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
            result.setdefault("returncode", process.returncode)
            result["elapsed_s"] = round(time.monotonic() - start, 3)
            connection.close()
            (destination / "result.json").write_text(json.dumps(result, indent=2) + "\n")
            print(json.dumps(result), flush=True)
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
