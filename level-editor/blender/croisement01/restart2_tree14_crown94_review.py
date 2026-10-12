"""Run bounded crown construction, saved views and exact native source audit."""
from pathlib import Path
import runpy
HERE=Path(__file__).resolve().parent
runpy.run_path(str(HERE/'restart2_tree14_crown94.py'),run_name='__main__')
runpy.run_path(str(HERE/'restart2_tree14_native_leaf_audit.py'),run_name='__main__')
