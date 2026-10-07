"""Attribute leaf endpoint pixels to the pinned physical scene before integration."""
from pathlib import Path
from collections import Counter
import sys,json,hashlib
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from refinement_review import _tree
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def load_scene():
 base=OUT/'restart2-textures/batch10-linked-static-v1/scene.blend';assert sha(base)=='493eb8afa1f3e60f433ee2faa5e018d65552292fe5e2dfb63e96a5eb32acfd68';bpy.ops.wm.open_mainfile(filepath=str(base));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'and not o.hide_render];substitutions=[('croisement02-north-woodland-bank','restart4-bank103-source-overlay-v2/model.blend','c0b8458ed39b0412fae16a03384c5585c3271cc5eb48072f0354380ab8f0072e'),('croisement02-tree-07','restart6-tree07-bark-fill-v1/native-restored-v1/worker.blend','6ba5b31c8e2f625e82535152ccea291f658e6f2e12cb53a1b539c488d8bc9189'),('croisement02-east-stone-wall-and-gate','restart7-fence-residual/wall101-appearance-reuse-v1/model.blend','3204d38f8b94a3ef78662b4633b7219b131ff8d61450d0fa8b8c562be64b0475'),('GROUND','restart4-fence14-ground-candidate-v1/model.blend','4f4875bc62b5602417830eb8b458bfbe8dcc9244095699096616fdb4dfb58bf8')];pins=[]
 for group,relative,digest in substitutions:
  path=OUT/relative;assert sha(path)==digest
  with bpy.data.libraries.load(str(path),link=False)as(src,dst):dst.scenes=src.scenes
  candidates=[]
  for imported_scene in dst.scenes:
   bpy.context.window.scene=imported_scene;bpy.context.view_layer.update()
   for obj in imported_scene.objects:
    if obj.type=='MESH'and ((group=='GROUND'and obj.name.startswith('Croisement02 Terrain'))or obj.get('asset_group')==group):candidates.append((obj,obj.matrix_world.copy()))
  assert candidates,(group,path);bpy.context.window.scene=scene;objects=[o for o in objects if not((group=='GROUND'and o.name.startswith('Croisement02 Terrain'))or o.get('asset_group')==group)]
  for obj,matrix in candidates:
   scene.collection.objects.link(obj);obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False;objects.append(obj)
  bpy.context.view_layer.update();pins.append(dict(group=group,model=str(path),sha256=digest,objects=[dict(name=o.name,matrix_world=[list(r)for r in m])for o,m in candidates]))
 return scene,objects,pins,base
