"""Use the shared FIFO render pool for Lincoln's legacy import path."""
import importlib.util
from pathlib import Path
import sys

_NAME = '_refinement_shared_render_slots'
if _NAME not in sys.modules:
    path = Path(__file__).resolve().parents[2] / 'refinement/render_slots.py'
    spec = importlib.util.spec_from_file_location(_NAME, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[_NAME] = module
    spec.loader.exec_module(module)

acquire = sys.modules[_NAME].acquire
release = sys.modules[_NAME].release
