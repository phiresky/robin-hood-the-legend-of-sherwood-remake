"""Build and inspect one private winch/room candidate under a shared render lease."""
from pathlib import Path
import runpy
import sys

HERE = Path(__file__).resolve().parent
for recipe, arguments in [
    ('restart2_winch_room_physical.py', []),
    ('restart2_winch_native.py', []),
    ('restart2_winch_room_probe_audit.py', ['--', 'winch-room-physical-v6']),
    ('restart2_winch_room_review.py', ['--', 'winch-room-physical-v6']),
    ('restart2_winch_room_review.py', ['--', 'winch-room-physical-v6', 'transition-00']),
]:
    sys.argv = [str(HERE / recipe), *arguments]
    runpy.run_path(str(HERE / recipe), run_name='__main__')
