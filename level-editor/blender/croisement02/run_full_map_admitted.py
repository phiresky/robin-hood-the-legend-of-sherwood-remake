"""FIFO-exclusive admission for the two frozen full-map browser diagnostics."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[3]
POOL = ROOT / 'level-editor/work/lincoln-refinement/render-slots'
# Current heavy-job admission explicitly uses acquire(2), not the module default.
ACTIVE_SLOTS = 2
BASE = ROOT / 'level-editor/work/croisement02-refinement/native-effect-production-oct12-v1'
RUNNERS = {
    'serial1': ('full-map-serial1-v2', '24b1b3b523a312ce1786d58937edd3051c92ef7bef7ca5bfb0c25760aefacb8d'),
    'baseline8': ('full-map-baseline8-v2', '5546f3643dd1593c4a64104ea26edea99aca0212f84441c33545e482336c7e18'),
}


def resources_ready():
    info = dict(line.split(':', 1) for line in Path('/proc/meminfo').read_text().splitlines())
    available = int(info['MemAvailable'].split()[0])
    browsers = []
    for path in Path('/proc').glob('[0-9]*/comm'):
        try:
            if path.read_text().strip() in {'chromium', 'chrome', 'chrome-headless'}:
                browsers.append(int(path.parent.name))
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            pass
    return available >= 10 * 1024**2 and not browsers, {'available_kib': available, 'visible_browser_pids': browsers}


def acquire_exclusive(pool, slots, ready, record):
    """Use the shared ticket protocol; never retain a partial set of slot locks."""
    pool.mkdir(parents=True, exist_ok=True)
    handles = [(pool / f'{i}.lock').open('a+') for i in range(slots)]
    handles.append((pool / 'browser.lock').open('a+'))
    queue = (pool / 'queue.lock').open('a+')
    ticket = None
    ticket_path = None
    admitted = False
    try:
        fcntl.flock(queue, fcntl.LOCK_EX)
        try:
            ticket_path = pool / f'{time.time_ns():020d}-{uuid.uuid4().hex}.ticket'
            ticket = ticket_path.open('x+')
            fcntl.flock(ticket, fcntl.LOCK_EX)
            ticket.write(str(os.getpid()) + '\n'); ticket.flush()
        finally:
            fcntl.flock(queue, fcntl.LOCK_UN)
        record('queued', {'ticket': ticket_path.name, 'slots': slots})
        last_notice = -30.0
        started = time.monotonic()
        while True:
            held = []
            detail = {'reason': 'earlier FIFO waiter'}
            fcntl.flock(queue, fcntl.LOCK_EX)
            try:
                live = []
                for path in sorted(pool.glob('*.ticket')):
                    if path == ticket_path:
                        live.append(path); continue
                    try:
                        candidate = path.open('r+')
                    except FileNotFoundError:
                        continue
                    with candidate:
                        try:
                            fcntl.flock(candidate, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        except BlockingIOError:
                            live.append(path)
                        else:
                            path.unlink(missing_ok=True)
                if live and live[0] == ticket_path:
                    allowed, detail = ready()
                    if allowed:
                        for handle in handles:
                            try:
                                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                            except BlockingIOError:
                                detail = {**detail, 'reason': 'existing slot/browser holder'}
                                break
                            held.append(handle)
                        if len(held) == len(handles):
                            for handle in handles:
                                handle.seek(0); handle.truncate(); handle.write(str(os.getpid()) + '\n'); handle.flush()
                            ticket_path.unlink()
                            admitted = True
                            record('admitted', detail)
                            return handles
                    else:
                        detail = {**detail, 'reason': 'memory reserve or visible browser'}
            finally:
                if not admitted:
                    for handle in held:
                        fcntl.flock(handle, fcntl.LOCK_UN)
                fcntl.flock(queue, fcntl.LOCK_UN)
            elapsed = time.monotonic() - started
            if elapsed - last_notice >= 30:
                record('waiting', {**detail, 'seconds': round(elapsed, 1)})
                last_notice = elapsed
            time.sleep(0.2)
    finally:
        if ticket_path: ticket_path.unlink(missing_ok=True)
        if ticket: ticket.close()
        queue.close()
        if not admitted:
            for handle in handles: handle.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--variant', choices=RUNNERS, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    directory, expected = RUNNERS[args.variant]
    runner = BASE / directory / 'run.mjs'
    actual = hashlib.sha256(runner.read_bytes()).hexdigest()
    if actual != expected: raise RuntimeError('Frozen runner hash changed')
    args.evidence.mkdir(parents=True, exist_ok=False)
    def record(event, detail):
        row = {'event': event, 'time_ns': time.time_ns(), 'pid': os.getpid(), **detail}
        with (args.evidence / 'admission.jsonl').open('a') as file: file.write(json.dumps(row) + '\n')
        print(json.dumps(row), flush=True)
    record('request', {'variant': args.variant, 'runner': str(runner), 'runner_sha256': actual, 'wrapper_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), 'pool': str(POOL), 'reserve_gib': 10, 'effective_slots': ACTIVE_SLOTS})
    child = None
    def interrupted(signum, _frame):
        record('signal', {'signal': signum})
        if child is not None and child.poll() is None:
            # Only the process group created by this wrapper is signalled.
            os.killpg(child.pid, signum)
            child.wait()
        raise SystemExit(128 + signum)
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    handles = acquire_exclusive(POOL, ACTIVE_SLOTS, resources_ready, record)
    try:
        if hashlib.sha256(runner.read_bytes()).hexdigest() != expected: raise RuntimeError('Runner changed while queued')
        child = subprocess.Popen(['node', str(runner)], cwd=ROOT, start_new_session=True, pass_fds=tuple(h.fileno() for h in handles))
        record('child-started', {'child_pid': child.pid})
        code = child.wait()
        record('child-exit', {'returncode': code})
        return code if code >= 0 else 128 - code
    finally:
        for handle in handles: handle.close()
        record('released', {})


if __name__ == '__main__':
    sys.exit(main())
