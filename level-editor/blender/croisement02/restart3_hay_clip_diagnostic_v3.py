"""Closed source-fitted hay mound with physical frayed straw, preserving canonical halves."""
import json,sys,shutil,math
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from scipy.ndimage import label,distance_transform_edt
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,scenery_workspace
from evidence_io import sha,write_json,record_recipe
from refinement_workspace import prepare,modified,validate,_geometry,_render
from source_projection_bake import bake
from tree_geometry import replace_mesh,SIN,COS,RAY
from scenery_geometry import Mesh
from render_tree import render_workspace
from render_slots import acquire,release

def main():
 asset='croisement02-south-field-haystack';old=scenery_workspace(asset);worker=OUT/'restart3-hay/clip-topology-research-v3/assets'/asset;geom=OUT/'restart3-hay/outline-v3/geometry.json';baseline_hash='39c50413bce3e9249866b3c34bd74a5ad2122c5656ddf5f9ac4489b72fea0b46'
 if worker.exists():raise FileExistsError(worker)
 if sha(old/'model.blend')!=baseline_hash:raise ValueError('Hay baseline changed')
 cfg=json.loads((old/'workspace.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0;objects=list(bpy.data.collections[cfg['collection_name']].all_objects);targets={o['source_node']:o for o in objects if o.type=='MESH' and o.get('asset_group')==asset};outside={o.name:_geometry(o,True) for o in objects if o not in targets.values()}
 if set(targets)!={'building-140','building-141'}:raise ValueError('Canonical halves differ')
 worker.mkdir(parents=True)
 plan=json.loads(geom.read_text());neutral=bpy.data.materials.new('Hay unknown rear');neutral.use_nodes=True;neutral.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.25,.25,.25,1);neutral.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value=1;trees=[];meshes={}
 for part in plan['parts']:
  m=Mesh();m.vertices=[tuple(v) for v in part['vertices']];m.faces=[tuple(f) for f in part['faces']];meshes[part['source_node']]=m;trees.append(BVHTree.FromPolygons([Vector(v) for v in m.vertices],m.faces))
 residual=np.array(Image.open(geom.parent/'remaining-straw.png').convert('L'))>0;core=np.array(Image.open(OUT/'restart2-vegetation/hay-outline-research/core-domain.png').convert('L'))>0;_,nearest=distance_transform_edt(~core,return_indices=True);components,count=label(residual);strands=[]
 def hit(x,y):
  origin=Vector((float(x),-float(y)/SIN,0))+RAY*5000;hits=[t.ray_cast(origin,-RAY) for t in trees];hits=[h for h in hits if h[0] is not None];return min(hits,key=lambda h:h[3])[0] if hits else None
 def point(x,y,z):return Vector((float(x),-(float(y)+z*COS)/SIN,z))
 for component in range(1,count+1):
  yy,xx=np.nonzero(components==component);pixels=np.column_stack((xx+860.5,yy+930.5));center=pixels.mean(axis=0);axis=np.array([1.,0.])
  if len(pixels)>1:
   values,vectors=np.linalg.eigh(np.cov((pixels-center).T));axis=vectors[:,np.argmax(values)]
  across=np.array([-axis[1],axis[0]]);transverse=(pixels-center)@across;bins=np.floor((transverse-transverse.min())/1.3).astype(int)
  # Every visible fleck is represented by a short closed straw segment. The
  # hidden connector follows the nearest native mound contour and is inferred.
  for binid in np.unique(bins):
   selected=pixels[bins==binid];c=selected.mean(axis=0);along=(selected-c)@axis;a=c+axis*(along.min()-.4);b=c+axis*(along.max()+.4);local=np.clip(np.rint(c-[860.5,930.5]).astype(int),[0,0],[189,179]);iy,ix=nearest[:,local[1],local[0]];native_near=np.array([ix+860.5,iy+930.5]);anchor=None
   for fraction in np.linspace(1.,.7,61):
    p=np.array(plan['radial_origin'])+(native_near-np.array(plan['radial_origin']))*fraction;anchor=hit(*p)
    if anchor is not None:break
   if anchor is None:raise ValueError('Frayed straw has no supporting mound')
   z=max(.45,float(anchor.z)+.25);pa,pb=point(*a,z),point(*b,z);part='building-140' if c[0]<plan['parameters'][4] else 'building-141';m=meshes[part];m.tube(pa,pb,.62,.46,n=6);near=pa if (pa-anchor).length<(pb-anchor).length else pb
   if (near-anchor).length>1.:m.tube(anchor,near,.48,.30,n=6)
   strands.append(dict(source_component=component,native_pixels=len(selected),source_endpoints=[a.tolist(),b.tolist()],world_anchor=list(anchor),inferred_height=z,source_node=part))
 topology={}
 for node,m in meshes.items():
  m.vertices=[tuple(Vector(v)+RAY*(-v[2]/SIN)) if v[2]<0 else v for v in m.vertices]
  topology[node]=replace_mesh(targets[node],m.vertices,m.faces,materials=[neutral])
  if topology[node]['nonmanifold_edges'] or topology[node]['degenerate_faces']:raise ValueError('Hay half not closed')
  for f in targets[node].data.polygons:f.use_smooth=f.index<512
 # Clip only the excess source silhouette from the coherent mound and its
 # attached strands. The cutter is a closed physical volume along native rays;
 # all retained interior depth and crown surface come from the v3 construction.
 mask=np.array(Image.open(OUT/'baseline/masks/000124.png').convert('L'))>0
 vertices=[];faces=[];corners={};active_cell=[0,0]
 def occupied(x,y):return 0<=y<mask.shape[0] and 0<=x<mask.shape[1] and bool(mask[y,x])
 def cv(x,y,side):
  a,b,c,d=[occupied(x+dx,y+dy) for dx,dy in [(-1,-1),(0,-1),(0,0),(-1,0)]];ambiguous=(a and c and not b and not d) or (b and d and not a and not c);key=(x,y,side,*active_cell) if ambiguous else (x,y,side)
  if key not in corners:
   xx=x+(active_cell[0]+.5-x)*.001 if ambiguous else x;yy=y+(active_cell[1]+.5-y)*.001 if ambiguous else y;corners[key]=len(vertices);p=Vector((878+xx,-(966+yy)/SIN,0))+RAY*(1000 if side else -1000);vertices.append(tuple(p))
  return corners[key]
 for yy,xx in zip(*np.nonzero(mask)):
  x,y=int(xx),int(yy);active_cell[:]=[x,y];q=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)]
  faces.append(tuple(cv(a,b,0) for a,b in reversed(q)));faces.append(tuple(cv(a,b,1) for a,b in q))
  for edge,(dx,dy) in enumerate([(0,-1),(1,0),(0,1),(-1,0)]):
   nx,ny=x+dx,y+dy
   if not (0<=ny<mask.shape[0] and 0<=nx<mask.shape[1] and mask[ny,nx]):
    a,b=q[edge],q[(edge+1)%4];faces.append((cv(*a,0),cv(*b,0),cv(*b,1),cv(*a,1)))
 cm=bpy.data.meshes.new('Private native hay contour cutter');cm.from_pydata(vertices,[],faces);cm.update();bm=bmesh.new();bm.from_mesh(cm);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bad=sum(not e.is_manifold for e in bm.edges);assert bad==0,('Cutter nonmanifold',bad);bmesh.ops.dissolve_limit(bm,angle_limit=.00001,verts=list(bm.verts),edges=list(bm.edges),delimit={'NORMAL'});assert all(e.is_manifold for e in bm.edges),'Simplified cutter invalid';print('Cutter closed',len(bm.verts),len(bm.faces),flush=True);bm.to_mesh(cm);bm.free();cutter=bpy.data.objects.new(cm.name,cm);bpy.context.scene.collection.objects.link(cutter)
 for node,obj in targets.items():
  bpy.context.view_layer.objects.active=obj;modifier=obj.modifiers.new('Observed outer straw contour','BOOLEAN');modifier.operation='INTERSECT';modifier.solver='EXACT';modifier.use_self=True;modifier.use_hole_tolerant=True;modifier.object=cutter;bpy.ops.object.modifier_apply(modifier=modifier.name)
  inverse=obj.matrix_world.inverted();clamped=0
  for vertex in obj.data.vertices:
   p=obj.matrix_world@vertex.co
   if p.z < -0.001:raise ValueError(('Unexpected below-ground clipped vertex',float(p.z)))
  obj.data.update();bm=bmesh.new();bm.from_mesh(obj.data);wire=[e for e in bm.edges if not e.link_faces];bmesh.ops.delete(bm,geom=wire,context='EDGES');loose=[v for v in bm.verts if not v.link_edges];bmesh.ops.delete(bm,geom=loose,context='VERTS');bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));result=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces),ground_clamped_vertices=clamped,removed_boolean_wire_edges=len(wire),removed_loose_vertices=len(loose))
  result['nonmanifold_details']=[dict(coordinates=[list(v.co) for v in e.verts],faces=len(e.link_faces)) for e in bm.edges if not e.is_manifold]
  bm.to_mesh(obj.data);bm.free();topology[node]=result;print('Clipped',node,{k:v for k,v in result.items() if k!='nonmanifold_details'},flush=True)
 bpy.data.objects.remove(cutter,do_unlink=True);bpy.data.meshes.remove(cm)
 write_json(worker/'topology.json',topology);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));print(topology)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
