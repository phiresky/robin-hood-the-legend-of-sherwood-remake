"""Render source-only wood fill packets from exact approved round-tree workers."""
from pathlib import Path
import sys,json,hashlib,shutil
import bpy
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_review import render_review
from refinement_workspace import _geometry
B=R/'level-editor/work/croisement02-refinement';A=B/'restart3-review-batches/three-tree-geometry-oct11-v1/user-approval.json'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def main(n):
 assert sha(A)=='a30e6aa6559e71347d7e1ee4b37a51defa2d4d880504ba7eeb334feafcef7d1e'
 decision=next(d for d in read(A)['decisions']if d['asset_id']==f'croisement02-tree-{n}');assert decision['decision']=='approved' and decision['scope']=='geometry'
 model=Path(decision['model']);assert sha(model)==decision['model_sha256'];lane=B/f'tree{n}-root-research';gallery=lane/'full-visible-geometry-review-oct11-v1/manifest.json';manifest=read(gallery)
 for f,h in manifest['files'].items():assert sha(Path(f))==h,f
 frames_path=lane/('full-visible-source-wood-oct11-v2'if n==32 else'full-visible-faired-source-wood-oct11-v1')/'views.json';frames=read(frames_path);projection=read(model.parent/'projection.json');output=B/f'approved-round-wood-textures-oct11-v1/tree{n}/packet'
 if output.exists():raise FileExistsError(output)
 acquire(slots=2)
 try:
  available=int(next(l.split()[1]for l in Path('/proc/meminfo').read_text().splitlines()if l.startswith('MemAvailable:')))*1024
  if available<6*1024**3 or shutil.disk_usage(R).free<10*1024**3:raise RuntimeError('RAM/disk guard')
  bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();collection=bpy.data.collections['Croisement02 Working'];wood=[o for o in collection.all_objects if o.type=='MESH'and o.get('asset_group')==decision['asset_id']and o.get('projection_component')!='crown'];assert len(wood)==(3 if n==32 else 1)
  before={o.name:_geometry(o,protect_appearance=True)for o in collection.all_objects if o.type=='MESH'};names=[o.name for o in wood]
  frames.update(scene_name=scene.name,object_names=names,render_object_names=sorted(names));frames.pop('render_object_names',None)
  for v in frames['views']:v.pop('camera_location',None);v.pop('camera_rotation_euler',None)
  rendered=render_review(output,scene_name=scene.name,collection_name=collection.name,asset_id=decision['asset_id'],source_path=projection['source'],frame_manifest=frames,lighting=frames['lighting'],source_mask_manifest=projection['source_mask_manifest'],render_object_names=names,allow_mask_revision=True)
  assert before=={o.name:_geometry(o,protect_appearance=True)for o in collection.all_objects if o.type=='MESH'}
  assert sha(model)==decision['model_sha256']
  report=dict(status='SOURCE_DERIVATIVE_REQUIRES_AGENT_REVIEW',asset_id=decision['asset_id'],model=str(model),model_sha256=decision['model_sha256'],user_geometry_approval=str(A),user_geometry_approval_sha256=sha(A),original_geometry_decision=decision,approved_camera_manifest=str(frames_path),approved_camera_manifest_sha256=sha(frames_path),geometry_and_appearance_unchanged=True,scope='Source-only derivative of approved wood and approved cameras; ground/crown omitted from material input only; no new geometry or inferred-source approval.',artifacts={str(p.relative_to(output)):sha(p)for p in output.rglob('*')if p.is_file()})
  (output/'derivation.json').write_text(json.dumps(report,indent=2)+'\n');print(output)
 finally:release()
if __name__=='__main__':main(int(sys.argv[sys.argv.index('--')+1]))
