"""Machine-wide pool limiting concurrent Blender renders and bakes.

Every map and shared tool acquires from the same lock directory, so the limit
holds across Lincoln, Nottingham and lossy-derivative jobs. Waiters are served
in FIFO order; a slot is one exclusive ``flock`` on ``<pool>/<index>.lock``,
which stays compatible with older per-map copies that lock the same files.
Pure Python (no bpy), so it is never part of a frozen Blender tooling snapshot.
"""
import fcntl
import os
from pathlib import Path
import time
import uuid

POOL = Path(__file__).resolve().parents[1] / 'work/needs-review/lincoln-refinement/render-slots'
SLOTS = 4
_lease = None


def acquire(slots=SLOTS, *, directory=None):
    """Hold one process-scoped render lease until exit; repeated calls are safe."""
    global _lease
    if _lease is not None:
        return
    root = Path(directory) if directory is not None else POOL
    root.mkdir(parents=True, exist_ok=True)
    handles = [(root / f'{i}.lock').open('a+') for i in range(slots)]
    queue = (root / 'queue.lock').open('a+')
    fcntl.flock(queue, fcntl.LOCK_EX)
    try:
        ticket_path = root / f'{time.time_ns():020d}-{uuid.uuid4().hex}.ticket'
        ticket = ticket_path.open('x+')
        fcntl.flock(ticket, fcntl.LOCK_EX)
        ticket.write(str(os.getpid()) + '\n')
        ticket.flush()
    finally:
        fcntl.flock(queue, fcntl.LOCK_UN)
    started = time.monotonic()
    last_notice = -30.0
    try:
        while True:
            # The ticket's live flock, rather than a PID lookup, identifies a
            # waiter across process namespaces and cleans up interrupted jobs.
            fcntl.flock(queue, fcntl.LOCK_EX)
            try:
                live = []
                for path in sorted(root.glob('*.ticket')):
                    if path == ticket_path:
                        live.append(path)
                        continue
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
                    for index, handle in enumerate(handles):
                        try:
                            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        except BlockingIOError:
                            continue
                        _lease = handle
                        handle.seek(0)
                        handle.truncate()
                        handle.write(str(os.getpid()) + '\n')
                        handle.flush()
                        ticket_path.unlink()
                        print(f'Render slot {index} acquired by {os.getpid()}', flush=True)
                        return
            finally:
                fcntl.flock(queue, fcntl.LOCK_UN)
            elapsed = time.monotonic() - started
            if elapsed - last_notice >= 30:
                print(f'Waiting for render slot ({elapsed:.0f}s)', flush=True)
                last_notice = elapsed
            time.sleep(0.2)
    finally:
        ticket_path.unlink(missing_ok=True)
        ticket.close()
        queue.close()
        for handle in handles:
            if handle is not _lease:
                handle.close()


def release():
    """Release the current asset's lease so queued work can run between bakes."""
    global _lease
    if _lease is not None:
        fcntl.flock(_lease, fcntl.LOCK_UN)
        _lease.close()
        _lease = None
        return True
    return False
