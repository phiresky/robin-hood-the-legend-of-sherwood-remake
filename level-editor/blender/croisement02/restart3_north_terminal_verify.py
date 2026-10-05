"""Run the bounded terminal checks in one FIFO lease, without saving the worker."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart3_north_terminal_source_front import main as source_front
from restart3_north_terminal_audit import main as contact
from restart3_north_terminal_rim_guard import main as rims
from restart3_north_terminal_contact_views import main as contact_views
from render_slots import acquire,release
if __name__=='__main__':
    acquire()
    try:
        source_front();contact();rims();contact_views()
    finally:release()
