"""Reopen the final kindling material derivative and validate its saved payload."""
import sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from audit_candidates import audit
from render_slots import acquire,release
from evidence_io import sha
acquire()
try:
 w=OUT/'restart3-kindling/native-rgb-v1/assets/croisement02-southwest-kindling-bundle';h=sha(w/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.view_layer.update();audit(w)
 if sha(w/'model.blend')!=h:raise ValueError('Audit changed saved model')
finally:release()
