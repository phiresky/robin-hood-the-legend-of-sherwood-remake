"""Bound private geometry/contact jobs before allocations and writes."""
from pathlib import Path
import shutil

def check(root, output, reserve=32*1024**2):
    free=shutil.disk_usage(root).free
    available=int(next(line.split()[1] for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:')))*1024
    used=sum(path.stat().st_size for path in output.rglob('*') if path.is_file())
    assert free>=10*1024**3+reserve, ('disk reserve',free)
    assert available>=10*1024**3, ('available memory',available)
    assert used<reserve, ('private output limit',used,reserve)
    return {'free_bytes':free,'available_memory_bytes':available,'output_bytes':used}

def guard_reader(reader, root, output):
    original=reader.guard
    def guarded(*args,**kwargs):
        check(root,output)
        return original(*args,**kwargs)
    reader.guard=guarded
