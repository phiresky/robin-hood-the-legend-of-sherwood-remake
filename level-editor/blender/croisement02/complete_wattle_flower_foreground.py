"""Add small native leaf fragments in front of the corrected wattle fence."""
import json,sys,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,scenery_workspace
from tree_geometry import SIN,COS,RAY,material,one_sided
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import _geometry

def main():
 original=scenery_workspace('croisement02-shrub-76');oldhash=sha(original/'model.blend');fence_path=OUT/'wattle99-source-candidate/v6/model.blend';fence_hash=sha(fence_path);joint=OUT/'wattle99-source-candidate/v6/flower-joint-v2';dst=OUT/'understory-candidates/native76-foreground-v2';dst.mkdir(exist_ok=False);observed=np.asarray(Image.open(OUT/'mixed-wood-audit/domain-502.png').convert('L'))>0;boundary_path=OUT/'mixed-wood-audit/boundary-roles76-93-v1/76-foliage76.png';boundary=np.asarray(Image.open(boundary_path).convert('L'))>0;assert boundary.sum()==22 and not np.any(observed&boundary);rgba=np.asarray(Image.open(joint/'first-hit-fence.png').convert('RGBA'))[1::3,1::3];plant=np.asarray(Image.open(joint/'plants-only.png').convert('RGBA'))[1::3,1::3,3]>127;blocked=(rgba[:,:,0]>245)&(rgba[:,:,1]<10)&(rgba[:,:,2]>245)&(rgba[:,:,3]>127);extra=boundary.copy();crop=observed[741:1026,526:792];extra[741:1026,526:792]|=crop&(~plant|blocked);known=extra&observed;assert int(known.sum())==140;source=np.asarray(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA')).copy();source[:,:,3]=extra*255;Image.fromarray(source).save(dst/'added-native-leaves.png');Image.fromarray(extra.astype('uint8')*255).save(dst/'added-domain.png');bpy.ops.wm.open_mainfile(filepath=str(fence_path));fence=next(o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-southwest-path-wattle-fence');tree=BVHTree.FromPolygons([fence.matrix_world@v.co for v in fence.data.vertices],[list(p.vertices)for p in fence.data.polygons]);bpy.ops.wm.open_mainfile(filepath=str(original/'model.blend'));scene=bpy.context.scene;old=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-shrub-76'];assert len(old)==3;before={o.name:_geometry(o,protect_appearance=True)for o in old};sites=[]
 for obj in old:
  ids=sorted({v for p in obj.data.polygons if obj.data.materials[p.material_index].get('foliage_observed')for v in p.vertices});points=np.array([tuple(obj.matrix_world@obj.data.vertices[i].co)for i in ids]);screen=np.column_stack((points[:,0],-points[:,1]*SIN-points[:,2]*COS));sites.append((obj,points,screen))
 mats=[material('76 observed foreground closure',dst/'added-native-leaves.png',True),material('76 inferred boundary front',dst/'added-native-leaves.png',False),material('76 inferred fragment backs',dst/'added-native-leaves.png',False)]
 for mat in mats:one_sided(mat)
 right=np.array([1.,0,0]);down=np.array([0.,-SIN,-COS]);ray=np.asarray(RAY);parts=[dict(vertices=[],faces=[],slots=[],uvs=[],colors=[],records=[])for _ in old]
 for sy,sx in zip(*np.nonzero(extra)):
  target=np.array([sx+.5,sy+.5]);nearest=[int(np.argmin(np.sum((screen-target)**2,axis=1)))for _,points,screen in sites];distances=[float(np.sum((screen[i]-target)**2))for (_,points,screen),i in zip(sites,nearest)];owner=int(np.argmin(distances));obj,points,screen=sites[owner];i=nearest[owner];center=points[i]+right*(target[0]-screen[i,0])+down*(target[1]-screen[i,1]);start_z=float(center[2]);hit,normal,face,distance=tree.ray_cast(Vector((target[0],-target[1]/SIN,0))+RAY*5000,-RAY,10000)
  if hit is not None:
   gap=float(np.dot(center-np.asarray(hit),ray))
   if gap<.6:center+=ray*(.6-gap)
  if center[2]<1:center+=ray*((1-center[2])/SIN)
  record=parts[owner];corners=[center+right*dx+down*dy for dx,dy in [(-.5,-.5),(.5,-.5),(.5,.5),(-.5,.5)]]
  if np.dot(np.cross(corners[1]-corners[0],corners[2]-corners[0]),ray)<0:corners.reverse()
  for back in [False,True]:
   pts=[p-ray*.02 for p in reversed(corners)]if back else corners;offset=len(record['vertices']);record['vertices'].extend([tuple(p)for p in pts]);record['faces'].append(tuple(range(offset,offset+4)));record['slots'].append(2 if back else(0 if observed[sy,sx]else 1));record['uvs'].extend((p[0]/1792,1-(-p[1]*SIN-p[2]*COS)/1152)for p in pts);record['colors'].extend([(1. if observed[sy,sx]and not back else 0.,1.,1.,1.)]*4)
  record['records'].append(dict(source_pixel=[int(sx),int(sy)],role='observed502 foreground closure'if observed[sy,sx]else'inferred22 boundary leaf',nearest_leaf_z=start_z,final_z=float(center[2])))
 additions=[]
 for (old_obj,_,_),record in zip(sites,parts):
  mesh=bpy.data.meshes.new(old_obj.name+' foreground leaf fragments');mesh.from_pydata(record['vertices'],[],record['faces']);mesh.update();obj=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(obj)
  for key in ['asset_group','asset_name','source_node','part_name']:obj[key]=old_obj.get(key,'Wattle flowering clumps76')
  for mat in mats:mesh.materials.append(mat)
  uv=mesh.uv_layers.new(name='Foliage UV');color=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=color
  for p,slot in zip(mesh.polygons,record['slots']):p.material_index=slot
  for i,(coord,c)in enumerate(zip(record['uvs'],record['colors'])):uv.data[i].uv=coord;color.data[i].color=c
  additions.append(dict(object=obj.name,vertices=len(mesh.vertices),faces=len(mesh.polygons),pixels=record['records']))
 assert before=={o.name:_geometry(o,protect_appearance=True)for o in old};bpy.ops.wm.save_as_mainfile(filepath=str(dst/'model.blend'));digest=sha(dst/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(dst/'model.blend'));assert before=={name:_geometry(bpy.context.scene.objects[name],protect_appearance=True)for name in before};assert sha(original/'model.blend')==oldhash and sha(fence_path)==fence_hash;write_json(dst/'validation.json',dict(status='PASS immutable original geometry/UV/materials; additive joint review pending',model_sha256=digest,original_model_sha256=oldhash,fence_model_sha256=fence_hash,source_boundary_sha256=sha(boundary_path),original_objects=list(before),objects=list(before)+[r['object']for r in additions],observed_pixels_added=int(known.sum()),inferred_boundary_pixels_added=int(boundary.sum()),additions=additions,depth_rule='Nearest own observed leaf vertex, advanced only where required to lie0.6 world unit in front of the corrected physical fence; ground clearance retained.',limitations=['Tiny added source fragments complete native foreground coverage; hidden depth is explicitly inferred.','Approved original leaf geometry and appearance remain exact.','Boundary22 source roles remain independent contextual inference, not a widened observed502 claim.','No catalog selection or approval changed.']));print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
