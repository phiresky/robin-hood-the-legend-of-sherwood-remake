"""Read-only native foreground gap attribution against approved receivers."""
import sys,json,math,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY,SIN,COS
from render_slots import acquire,release
from evidence_io import sha,write_json
from restart4_stump_final_contact import frame
ROOT=OUT/'restart6-source-coverage'
SPECS={39:(OUT/'restart2-textures/approved7-combined-fill-v1/croisement02-tree-39/native-front-preparation/experiment/bake-v1/worker.blend','4487f04a0b69537b836fe3681c3afc60ee8e2c39a5ece968c6e791f666861553'),95:(OUT/'restart2-textures/approved-fence95-fill-v1/croisement02-east-upright-rail-fence-95/experiment/bake-v1/worker.blend','0de98cd7be4b9275e2eef0cde42fc034ee529ece6ab5aedc994cd137a9d396a6')}
def main(index):
 assert shutil.disk_usage(OUT).free>25*2**30;source,digest=SPECS[index];assert sha(source)==digest;out=ROOT/f'baseline-audit-{index}-v3';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();scene=bpy.context.scene;asset=f'croisement02-tree-{index:02d}'if index!=95 else'croisement02-east-upright-rail-fence-95'
 for obj in scene.objects:
  if obj.type=='MESH'and obj.get('asset_group')!=asset:obj.hide_render=True
 objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset];assert objects;records=[];wood=[];verts=[];world=[];triangles=[];owners=[]
 for o in objects:
  points=np.array([o.matrix_world@v.co for v in o.data.vertices]);projected=np.column_stack((points[:,0],-points[:,1]*SIN-points[:,2]*COS));records.append(dict(object=o.name,source_node=o.get('source_node'),properties={k:str(o[k])for k in o.keys()},vertices=len(points),faces=len(o.data.polygons),bounds=[points.min(0).tolist(),points.max(0).tolist()],source_bounds=[projected.min(0).tolist(),projected.max(0).tolist()],materials=[m.name if m else None for m in o.data.materials]))
  if o.get('projection_component')=='crown' or o.get('tree_part')=='crown' or 'crown'in o.name.lower() or 'canopy'in o.name.lower():o.hide_render=True;continue
  wood.append(o);o.data.calc_loop_triangles();base=len(verts);verts.extend([Vector((x,y,0))for x,y in projected]);world.extend([Vector(p)for p in points]);
  for t in o.data.loop_triangles:
   indices=tuple(base+j for j in t.vertices);a,b,c=[verts[j]for j in indices]
   if abs((b-a).cross(c-a).z)<1e-5:continue
   triangles.append(indices);owners.append(o.name)
 tree=BVHTree.FromPolygons(verts,triangles,all_triangles=True);mask=np.array(Image.open(ROOT/f'source-audit-v1/exposed-{index}.png'))>0;rows=[]
 for y,x in np.argwhere(mask):
  q,n,i,d=tree.find_nearest(Vector((x+.5,y+.5,0)));t=triangles[i];p=barycentric_transform(q,*[verts[j]for j in t],*[world[j]for j in t]);rows.append(dict(pixel=[int(x),int(y)],nearest_object=owners[i],projected_distance=float(d),nearest_source=[q.x,q.y],nearest_world=list(p)))
 write_json(out/'report.json',dict(source=str(source),source_sha256=digest,objects=records,pixels=rows,wood_objects=[o.name for o in wood],scope='Nearest projected approved physical surface; no geometry or source ownership change.'))
 yy,xx=np.where(mask);box=[int(xx.min()-20),int(yy.min()-20),int(xx.max()+21),int(yy.max()+21)];cx=(box[0]+box[2])/2;cy=(box[1]+box[3])/2;scale=max(box[2]-box[0],box[3]-box[1]);center=Vector((cx,-cy/SIN,0));camera=bpy.data.objects.new('Native source gap camera',bpy.data.cameras.new('Native source gap camera'));scene.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.ortho_scale=scale;camera.data.clip_end=20000;camera.location=center+RAY*5000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.resolution_x=512;scene.render.resolution_y=512;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA';scene.render.image_settings.file_format='PNG';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';bpy.context.view_layer.update();scene.render.filepath=str(out/'native-wood.png');bpy.ops.render.render(write_still=True);box=[cx-scale/2,cy-scale/2,cx+scale/2,cy+scale/2];native=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').transform((512,512),Image.Transform.EXTENT,box,Image.Resampling.NEAREST);native.save(out/'native-source.png');sheet=Image.new('RGBA',(1536,512),(35,35,35,255));sheet.paste(native,(0,0));render=Image.open(out/'native-wood.png').convert('RGBA');sheet.paste(Image.alpha_composite(native,render),(512,0));sheet.paste(render,(1024,0));sheet.save(out/'source-comparison.png');write_json(out/'camera.json',dict(box=box,camera=[list(r)for r in camera.matrix_world],scale=scale));assert sha(source)==digest
 print(index,'max nearest distance',max(r['projected_distance']for r in rows),flush=True)
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
