"""Small shared primitives for frozen runtime evidence and owned child cleanup."""
import hashlib
import os
from pathlib import Path
import shutil
import signal
import subprocess


class DriverInterrupted(RuntimeError):
    """Not an OSError: polling loops must never swallow cancellation/alarm."""


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def snapshot_executable(source, target):
    """Copy from one open descriptor; Cargo replacing its path cannot change it."""
    with Path(source).open("rb") as stream:
        expected = hashlib.file_digest(stream, "sha256").hexdigest()
        stream.seek(0)
        with Path(target).open("xb") as destination:
            shutil.copyfileobj(stream, destination)
        if digest(target) != expected:
            raise ValueError("runner changed while creating isolated executable snapshot")
    Path(target).chmod(0o555)
    return expected


def observe_client_source(source):
    """Diagnostic build-tree observation, never authority for a later launch."""
    try:
        return {"path": str(source), "sha256": digest(source)}
    except OSError as error:
        return {"path": str(source), "error_type": type(error).__name__, "error": str(error)}


def snapshot_client(source, evidence, summary):
    destination = evidence / "bin"
    destination.mkdir()
    binary = destination / source.name
    summary["binary_source"] = str(source)
    summary["binary"] = str(binary)
    summary["binary_sha256"] = snapshot_executable(source, binary)
    return binary


def verify_client(binary, summary):
    if digest(binary) != summary["binary_sha256"]:
        raise RuntimeError("retained game executable changed before launch")


def stop_process_group(child):
    """Reap a start_new_session child and stop descendants even after its exit."""
    for signum in (signal.SIGCONT, signal.SIGTERM):
        try:
            os.killpg(child.pid, signum)
        except ProcessLookupError:
            pass
    try:
        child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(child.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    child.wait(timeout=5)


def finish_children(children, summary):
    """One cleanup failure must not prevent the remaining children or report."""
    failures = []
    for child in reversed(children):
        try:
            stop_process_group(child)
        except Exception as error:
            failures.append({"pid": child.pid, "error": str(error)})
    if failures:
        summary.update(completed=False, cleanup_errors=failures)


def failure(summary, error):
    summary.update(completed=False, error=str(error), error_type=type(error).__name__)
