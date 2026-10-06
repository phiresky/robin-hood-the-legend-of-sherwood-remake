"""Bind final source-preserving rigs to full native-first sheets and attachment metrics."""
import sys,json,math,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import RAY,SIN,COS
from refinement_review import _tree
from render_slots import acquire,release
ROOT=OUT/'restart5-initial-nets'
def main():
 assert shutil.disk_usage(OUT).free>25*1024**3
 for key in ['00','01']:
  w=ROOT/f'candidate-v5/profile-{key}';report=json.loads((w/'report.json').read_text());model=w/'model.blend';assert sha(model)==report['model_sha256'];support=report['support'];bpy.ops.wm.open_mainfile(filepath=support['model']);bpy.context.view_layer.update();wood=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==support['asset_id']and o.get('projection_component')!='crown'and 'crown'not in o.name.lower()];wt,_,_=_tree(wood)
  bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;own=[o for o in scene.objects if o.type=='MESH'];tie=bpy.data.objects['Inferred upper fastening loop'];line=bpy.data.objects['Initial lifting line'];gaps=[wt.find_nearest(tie.matrix_world@v.co)[3]for v in tie.data.vertices];tt,_,_=_tree([tie]);center=sum((v.co for v in line.data.vertices[:8]),Vector())/8;p,n,i,d=tt.find_nearest(center);attachment=dict(tie_wood_sample_gap_min=min(gaps),tie_wood_sample_gap_median=float(np.median(gaps)),tie_wood_sample_gap_max=max(gaps),line_endpoint_inside_tie=bool((center-p).dot(n)<=0),line_endpoint_tie_surface_distance=d,definition='Loop wraps exact wood cross-section with thin clearance; endpoint lies within closed loop tube. Distances are mesh-vertex nearest-surface samples, not exhaustive contact area.');assert attachment['line_endpoint_inside_tie']
  assert not(w/'actual-sheet.png').exists();scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=64;cam=scene.camera;pts=[o.matrix_world@v.co for o in own for v in o.data.vertices];center=(Vector(tuple(min(p[i]for p in pts)for i in range(3)))+Vector(tuple(max(p[i]for p in pts)for i in range(3))))/2;scale=max((p-center).length for p in pts)*2.25;solid=bpy.data.materials.new('Solid final rig review');solid.use_nodes=True;solid.diffuse_color=(.5,.5,.5,1);solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.5,.5,.5,1);cameras=[]
  for view in range(8):
   a=-math.pi/2+view*math.pi/4;direction=Vector((math.cos(a)*COS,math.sin(a)*COS,SIN));cam.location=center+direction*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=scale;cameras.append(dict(view=view,direction=list(direction),ortho_scale=scale))
   for mode in ['actual','solid']:
    scene.view_layers[0].material_override=solid if mode=='solid'else None;scene.render.filepath=str(w/f'final-{view}-{mode}.png');bpy.ops.render.render(write_still=True)
  for mode in ['actual','solid']:
   sheet=Image.new('RGB',(1536,768),(30,30,30))
   for i in range(8):
    im=Image.open(w/f'final-{i}-{mode}.png').convert('RGBA');sheet.paste(im,((i%4)*384,(i//4)*384),im)
   sheet.save(w/f'{mode}-sheet.png')
  write_json(w/'final-review-evidence.json',dict(model_sha256=report['model_sha256'],attachment=attachment,cameras=cameras,actual_sha256=sha(w/'actual-sheet.png'),solid_sha256=sha(w/'solid-sheet.png'),source_comparison_sha256=sha(w/'source-comparison.png')));assert sha(model)==report['model_sha256']
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
