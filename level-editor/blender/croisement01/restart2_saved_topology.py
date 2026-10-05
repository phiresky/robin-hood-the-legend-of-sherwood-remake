"""Record saved topology after an already completed source/contact review."""
import sys,json
from pathlib import Path
import bpy
sys.path.insert(0,str(Path(__file__).parent))
import restart2_tree_audit as audit
from render_slots import acquire
from evidence_io import sha
worker=Path(sys.argv[sys.argv.index('--')+1]).resolve()
def reopen_reviewed_worker():
    acquire()
    report=json.loads((worker/'inspection/native-geometry-coverage/report.json').read_text())
    assert sha(worker/'model.blend')==report['model_sha256']
    assert not (worker/'inspection/saved-tree-geometry.json').exists()
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
audit.audit_native_coverage.main=reopen_reviewed_worker
audit.main()
