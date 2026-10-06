"""Preserve a bounded native contact appearance on the unchanged bank surface."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from restore_ground75_source import geometry
from restart3_initial_fence_contact import link
from refinement_review import _tree
from review_bank_candidate import camera
D=OUT/'restart4-bank103-source-overlay-v2'
BANK=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
def clip(poly,axis,edge,lower):
 result=[]
 for a,b in zip(poly,poly[1:]+poly[:1]):
  ia=a[axis]>=edge if lower else a[axis]<=edge;ib=b[axis]>=edge if lower else b[axis]<=edge
  if ia:result.append(a)
  if ia!=ib:result.append(a+(b-a)*((edge-a[axis])/(b[axis]-a[axis])))
 return result
def main():
 assert not D.exists();D.mkdir();assert sha(BANK)=='69ecb7b704e30d6d64565a44aa810a21b924195609dbe7ac35818a0209137641'
 proposal=json.loads((OUT/'restart4-bank131-current-audit-v2/restoration-proposal.json').read_text());coords=proposal['coordinates'];assert len(coords)==103
 bpy.ops.wm.open_mainfile(filepath=str(BANK));bpy.context.view_layer.update()
 bank=[o for o in bpy.data.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-north-woodland-bank'];signatures={o.name:geometry(o)for o in bank}
 def images():
  result={}
  for o in bank:
   for m in o.data.materials:
    for n in m.node_tree.nodes:
     if n.type=='TEX_IMAGE'and n.image:
      a=np.empty(len(n.image.pixels),np.float32);n.image.pixels.foreach_get(a);result[n.image.name]=hashlib.sha256(a.tobytes()).hexdigest()
  return result
 old_images=images();part=next(o for o in bank if o.name.endswith('part 000'));part.data.calc_loop_triangles()
 world=np.array([list(part.matrix_world@v.co)for v in part.data.vertices]);projected=np.column_stack((world[:,0],-world[:,1]*SIN-world[:,2]*COS));tris=[list(t.vertices)for t in part.data.loop_triangles]
 tri2=projected[tris];lo=tri2.min(1);hi=tri2.max(1);verts=[];faces=[];uvs=[];coverage=[]
 x0=min(x for x,y in coords);y0=min(y for x,y in coords);x1=max(x for x,y in coords)+1;y1=max(y for x,y in coords)+1
 for x,y in coords:
  area=0
  for i in np.where((lo[:,0]<x+1)&(hi[:,0]>x)&(lo[:,1]<y+1)&(hi[:,1]>y))[0]:
   q=[v.copy()for v in tri2[i]]
   for ax,edge,low in [(0,x,True),(0,x+1,False),(1,y,True),(1,y+1,False)]:
    if q:q=clip(q,ax,edge,low)
   if len(q)<3:continue
   ar=abs(sum(a[0]*b[1]-a[1]*b[0]for a,b in zip(q,q[1:]+q[:1])))/2
   if ar<1e-8:continue
   # Keep only the visible upward bank top, excluding projected rear faces.
   t=world[tris[i]];normal=np.cross(t[1]-t[0],t[2]-t[0])
   if normal[2]<=0:continue
   mat=np.column_stack((tri2[i][1]-tri2[i][0],tri2[i][2]-tri2[i][0]));start=len(verts)
   for pixel in q:
    w=np.linalg.solve(mat,pixel-tri2[i][0]);v=t[0]+w[0]*(t[1]-t[0])+w[1]*(t[2]-t[0]);verts.append(tuple(v+np.array(RAY)*.002));uvs.append(((pixel[0]-x0)/(x1-x0),1-(pixel[1]-y0)/(y1-y0)))
   faces.append(tuple(range(start,len(verts))));area+=ar
  coverage.append(area)
 assert all(abs(a-1)<1e-5 for a in coverage),coverage
 src=np.array(Image.open(OUT/'source-states/covered.png').convert('RGBA'));crop=src[y0:y1,x0:x1].copy();crop[:,:,3]=0
 for x,y in coords:crop[y-y0,x-x0]=src[y,x]
 Image.fromarray(crop).save(D/'native103.png');im=bpy.data.images.load(str(D/'native103.png'));im.pack()
 m=part.data.materials[0].copy();m.name='Native103 bank contact source';n=next(n for n in m.node_tree.nodes if n.type=='TEX_IMAGE');n.image=im;n.interpolation='Closest';n.extension='CLIP'
 for edge in list(n.inputs['Vector'].links):m.node_tree.links.remove(edge)
 uvnode=m.node_tree.nodes.new('ShaderNodeUVMap');uvnode.uv_map='Native crop';m.node_tree.links.new(uvnode.outputs['UV'],n.inputs['Vector'])
 mesh=bpy.data.meshes.new('Exact source103 clipped bank surface');mesh.from_pydata(verts,[],faces);mesh.materials.append(m);uv=mesh.uv_layers.new(name='Native crop')
 for poly in mesh.polygons:
  for li in poly.loop_indices:uv.data[li].uv=uvs[mesh.loops[li].vertex_index]
 overlay=bpy.data.objects.new('North Woodland Bank / Native103 contact appearance',mesh);bpy.context.scene.collection.objects.link(overlay);overlay['asset_group']='croisement02-north-woodland-bank';overlay['source_role']='Conservative native contact appearance; uncertain wood/shadow boundary';overlay['source_overlay_offset']=.002
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(D/'model.blend'),compress=True);bpy.ops.wm.open_mainfile(filepath=str(D/'model.blend'));bpy.context.view_layer.update();bank=[bpy.data.objects[n]for n in signatures];assert all(geometry(o)==signatures[o.name]for o in bank);assert images()==old_images
 overlay=bpy.data.objects['North Woodland Bank / Native103 contact appearance'];scene=bpy.data.scenes.new('Bank103 current contact');bpy.context.window.scene=scene
 for o in bank+[overlay]:link(scene,o);o.hide_render=False
 auth=json.loads((OUT/'restart2-textures/batch-v9-texture-approval-v1/approved-addition-authority.json').read_text());wood=json.loads((OUT/'restart2-textures/tree06-retained-wood-fill-v1/approved-preparation/experiment/original-atlas-bake-v2/assembly-authority.json').read_text());context=[]
 def load(p,names):
  with bpy.data.libraries.load(str(p),link=False)as(s,d):d.objects=names
  for o in d.objects:link(scene,o);o.hide_render=False;context.append(o)
 load(OUT/'restart4-fence14-ground-candidate-v1/model.blend',['Croisement02 Terrain']);load(auth['base']['path'],['Northwest Tree 06 / Crown']);load(wood['wood_derivative']['path'],wood['wood_derivative']['only_receivers']);load(auth['approved_texture_model']['path'],[auth['addition_object']]);bpy.context.view_layer.update()
 root=bpy.data.objects[auth['addition_object']];v=[o.matrix_world@v.co for o in bank for v in o.data.vertices];bt=[];offset=0
 for o in bank:
  o.data.calc_loop_triangles();bt.extend(tuple(offset+j for j in t.vertices)for t in o.data.loop_triangles);offset+=len(o.data.vertices)
 bv=BVHTree.FromPolygons(v,bt,all_triangles=True);gaps=[]
 for vertex in root.data.vertices:
  p=root.matrix_world@vertex.co;hit,_,_,_=bv.ray_cast(Vector((p.x,p.y,1000)),Vector((0,0,-1)))
  if hit:gaps.append((p.z-hit.z,list(p),list(hit)))
 support=dict(vertex_samples=len(gaps),buried_or_contact=sum(g[0]<=.01 for g in gaps),elevated=sum(g[0]>.01 for g in gaps),minimum_gap=min(g[0]for g in gaps),maximum_gap=max(g[0]for g in gaps),lowest_structural_vertices=sorted(gaps,key=lambda g:g[1][2])[:12],interpretation='Some projecting tips can remain elevated; count and lowest structural contacts are reported without altering approved root geometry.')
 tree,owners,_=_tree(bank+[overlay]+context);rows=[]
 for x,y in coords:
  hit,_,i,_=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*10000,-RAY);rows.append(dict(pixel=[x,y],owner=owners[i].name));assert owners[i]==overlay
 for name,direction in [('native',RAY),('oblique',Vector((.55,-.65,.5)).normalized())]:
  camera(scene,Vector((660,-536/SIN,0))+RAY*(44/RAY.z),direction,768,576,140);scene.cycles.transparent_max_bounces=512
  for label,hidden in [('before',True),('after',False)]:overlay.hide_render=hidden;scene.render.filepath=str(D/f'{name}-{label}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
 overlay.hide_render=False
 write_json(D/'validation.json',dict(status='Private exact source overlay; visual review pending',model_sha256=sha(D/'model.blend'),bank_sha256=sha(BANK),native_samples=103,coverage_area_per_sample=coverage,original_geometry_uv_exact=True,original_material_images_exact=True,offset_along_source_ray=.002,no_geometry_outside103=True,source_crop_sha256=sha(D/'native103.png'),first_hits=rows,root_support=support,atlas_sharing='Existing bank material is linear-filtered; changing any used texel affects neighboring surface samples. Explicit clipped source overlay avoids every existing atlas write.',api_calls=0))
 print(support,flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
