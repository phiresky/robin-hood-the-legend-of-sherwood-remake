"""Create a private inferred wicker-tip candidate for source-supported edge pixels."""
import sys,json,math,hashlib,shutil
from pathlib import Path
import bpy
from mathutils import Vector
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_review import render_review
B=R/'level-editor/work/croisement02-refinement';O=B/'static-terrain-closure-oct11-v1/fence100-tip-candidate-v2';A='croisement02-southwest-field-wattle-fence'
MODEL=B/'texture-fill-round-1'/A/'experiment/bake-v1/worker.blend'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 acquire(slots=2)
 try:
  assert shutil.disk_usage(R).free>=10*1024**3
  assert int(next(l.split()[1]for l in Path('/proc/meminfo').read_text().splitlines()if l.startswith('MemAvailable:')))*1024>=6*1024**3
  assert sha(MODEL)=='9609476357f969702d754fe7c3ac7054f9eafb9609cd7d6f655b0bc0acccef71'
  O.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(MODEL));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2
  collection=bpy.data.collections['Croisement02 Working'];base=[o for o in collection.all_objects if o.type=='MESH'and o.get('asset_group')==A]
  assert len(base)==2
  s=math.sin(math.radians(35));c=math.cos(math.radians(35));cy=-(1034.5+28*c)/s
  verts=[];faces=[];rings=[(569.2,1.2),(570.8,1.25),(572.6,1.25),(573.35,.95),(573.9,.75)]
  for x,radius in rings:
   for j in range(10):
    angle=2*math.pi*j/10;verts.append((x,cy+radius*math.cos(angle),28+radius*math.sin(angle)))
  for k in range(len(rings)-1):
   for j in range(10):a=k*10+j;b=k*10+(j+1)%10;faces.append((a,b,b+10,a+10))
  faces.extend([tuple(reversed(range(10))),tuple(range(40,50))]);mesh=bpy.data.meshes.new('Source100 elevated wicker tip');mesh.from_pydata(verts,[],faces);mesh.update();obj=bpy.data.objects.new('Source100 elevated wicker tip',mesh);collection.objects.link(obj)
  obj['asset_group']=A;obj['source_node']='building-023';obj['projection_component']='source100-tip-candidate';obj['inference_scope']='Small elevated wicker-tip continuation; no ground role transfer. Exact six native centers source-supported; rear depth inferred.'
  mat=bpy.data.materials.new('Unapproved inferred wicker tip neutral');mat.diffuse_color=(.3,.3,.3,1);obj.data.materials.append(mat)
  bpy.context.view_layer.update();bpy.ops.wm.save_as_mainfile(filepath=str(O/'model.blend'))
  (O/'candidate.json').write_text(json.dumps(dict(status='PRIVATE_GEOMETRY_HYPOTHESIS_REQUIRES_REVIEW',baseline=str(MODEL),baseline_sha256=sha(MODEL),model_sha256=sha(O/'model.blend'),asset=A,source_node='building-023',native_centers=[[571,1033],[571,1034],[571,1035],[572,1034],[572,1035],[573,1034]],height=28,inferred_depth='Approximately2.8 units, narrowed toward tip',topology='Separate closed overlapping tip; connectivity must be reviewed before eligibility',scope='Geometry only; neutral new material. Approved base geometry/material unchanged.',canonical_writes=0),indent=2)+'\n')
  render_review(O/'review',scene_name=scene.name,collection_name=collection.name,asset_id=A,source_path=B/'source-states/covered.png',width=256,height=256,render_object_names=[o.name for o in base]+[obj.name],allow_projection_revision=True,allow_mask_revision=True)
 finally:release()
if __name__=='__main__':main()
