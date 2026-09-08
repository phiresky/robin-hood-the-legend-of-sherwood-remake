#!/usr/bin/env python3
"""Drive bounded terminal-flow diagnostics on an already-running isolated client.

Usage: client_modal_flow.py URL EVIDENCE_DIR early-win|win|loss
Use client_soak.py to launch; DISPLAY must match that client. This uses normal
X11 Return events plus the explicit diagnostic WIN/LOOSE console commands.
It does not claim victory by ordinary play. The non-early cases wait for
simulation progression beyond startup briefing before requesting termination.
"""
import json
from pathlib import Path
import subprocess
import sys
import time
import urllib.request

url, folder, scenario = sys.argv[1:]
if scenario not in ("early-win", "win", "loss"):
    raise SystemExit("scenario must be early-win, win, or loss")
folder = Path(folder)
folder.mkdir(parents=True, exist_ok=True)
input_script = Path(__file__).with_name("client_x11.py")
deadline = time.monotonic() + 180

def snapshot():
    with urllib.request.urlopen(url + "/host-debug", timeout=3) as response:
        return json.load(response)

def enter():
    subprocess.run([sys.executable, str(input_script), "key", "Return"], check=True,
                   stdout=subprocess.DEVNULL)

def capture(name):
    subprocess.run(["import", "-window", "root", str(folder / (name + ".png"))], check=True)

with (folder / "driver.jsonl").open("w") as output:
    while True:
        if time.monotonic() > deadline:
            raise SystemExit("client never became ready")
        try:
            before = snapshot()
            break
        except Exception:
            time.sleep(1)
    capture("ready")
    if scenario == "early-win":
        enter()
    else:
        while before["frame"] < 8:
            if time.monotonic() > deadline:
                raise SystemExit("briefing did not release the simulation")
            enter()
            time.sleep(1)
            before = snapshot()
    output.write(json.dumps({"before_terminal": before, "scenario": scenario}) + "\n")
    command = "LOOSE" if scenario == "loss" else "WIN"
    request = urllib.request.Request(url + "/console", method="POST",
        data=json.dumps({"command": command}).encode(),
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=5) as response:
        output.write(json.dumps({"console_response": json.load(response)}) + "\n")
    output.flush()
    for index in range(14):
        time.sleep(1)
        log = folder / "client.log"
        if log.exists() and "game future returned, exit_code=0" in log.read_text():
            output.write(json.dumps({"terminal_exit_completed": True}) + "\n")
            output.flush()
            break
        enter()
        try:
            state = snapshot()
        except Exception as error:
            state = {"error": str(error)}
        output.write(json.dumps({"confirmation": index, "state": state}) + "\n")
        output.flush()
        capture(f"confirmation-{index:02}")
    print(f"{scenario}: confirmations complete; inspect screenshots and client log for transition outcome")
