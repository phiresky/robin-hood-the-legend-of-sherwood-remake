"""Private complete adjacent-bank surface hypothesis with protected root rays."""
import argparse,collections,hashlib,json,math,shutil,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
p=argparse.ArgumentParser();p.add_argument('--tree',type=int,choices=[4,5],required=True);p.add_argument('--revision',type=int,required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);R=ROOT/'level-editor/work/croisement01-refinement/restart2';tree_id=a.tree;source_revision={4:6,5:7}[tree_id];source=R/f'tree{tree_id:02d}-v{source_revision}/assets/croisement01-tree-{tree_id:02d}';dest=R/f'tree{tree_id:02d}-complete-banks-v{a.revision}';worker=dest/f'assets/croisement01-tree-{tree_id:02d}';assert not dest.exists();acquire();bpy.ops.wm.open_mainfile(filepath=str(source/'model.blend'));bpy.context.preferences.filepaths.save_version=0;cfg=json.loads((source/'workspace.json').read_text());collection=bpy.data.collections[cfg['collection_name']];nodes=([f'building-{i:03d}' for i in range(2,6)] if tree_id==4 and a.revision>=4 else {4:['building-003','building-004'],5:['building-006','building-007']}[tree_id]);banks=[o for o in collection.all_objects if o.type=='MESH' and o.get('source_node') in nodes];source_hash=sha(source/'model.blend')
def geom(o):return hashlib.sha256(json.dumps([[list(v.co) for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons]],separators=(',',':')).encode()).hexdigest()
def make_bvh(objects):
 points=[];faces=[]
 for o in objects:
  off=len(points);points.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(off+k for k in f.vertices) for f in o.data.polygons)
 return BVHTree.FromPolygons(points,faces),points
protected={o.get('source_node'):geom(o) for o in collection.all_objects if o.type=='MESH'};points=[];faces=[];owners=[]
for o in banks:
 off=len(points);points.extend(o.matrix_world@v.co for v in o.data.vertices)
 for f in o.data.polygons:
  normal=(o.matrix_world.to_3x3().inverted().transposed()@f.normal).normalized()
  if normal.z>.1:faces.append(tuple(off+k for k in f.vertices));owners.append(o.get('source_node'))
assert faces;top_tree=BVHTree.FromPolygons(points,faces);step=4.;xs=np.arange(math.floor(min(p.x for p in points)/step)*step-step,math.ceil(max(p.x for p in points)/step)*step+step*1.1,step);ys=np.arange(math.floor(min(p.y for p in points)/step)*step-step,math.ceil(max(p.y for p in points)/step)*step+step*1.1,step);z0=np.zeros((len(xs),len(ys)));domain=np.zeros_like(z0,dtype=bool);labels=np.full_like(z0,-1,dtype=int)
for i,x in enumerate(xs):
 for j,y in enumerate(ys):
  hit,_,index,_=top_tree.ray_cast(Vector((x,y,5000)),Vector((0,0,-1)),10000)
  if hit is not None:domain[i,j]=True;z0[i,j]=hit.z;labels[i,j]=nodes.index(owners[index])
# Small raster gaps between adjoining native faces are inferred as continuous
# soil. The outer domain remains the complete bank footprint, not a rectangle.
def dilate(m):
 p=np.pad(m,1);return np.logical_or.reduce([p[1+di:1+di+m.shape[0],1+dj:1+dj+m.shape[1]] for di,dj in [(0,0),(1,0),(-1,0),(0,1),(0,-1)]])
def erode(m):return ~dilate(~m)
closed=domain.copy()
for _ in range(2):closed=dilate(closed)
for _ in range(2):closed=erode(closed)
if (tree_id==4 and a.revision>=2) or (tree_id==5 and a.revision>=4):
 # The selected banks are disconnected archaeological proxy footprints.
 # Bridge their intervening soil with a separately reviewed inferred slope.
 coords=sorted(set((int(i),int(j)) for i,j in np.argwhere(domain)))
 def cross(o,a,b):return (a[0]-o[0])*(b[1]-o[1])-(a[1]-o[1])*(b[0]-o[0])
 lower=[];upper=[]
 for point in coords:
  while len(lower)>=2 and cross(lower[-2],lower[-1],point)<=0:lower.pop()
  lower.append(point)
 for point in reversed(coords):
  while len(upper)>=2 and cross(upper[-2],upper[-1],point)<=0:upper.pop()
  upper.append(point)
 hull=lower[:-1]+upper[:-1];image=Image.new('L',(len(ys),len(xs)));ImageDraw.Draw(image).polygon([(j,i) for i,j in hull],fill=255);closed=np.asarray(image)>0
closed[[0,-1]]=False;closed[:,[0,-1]]=False
filled=closed&~domain
for _ in range(max(len(xs),len(ys))):
 pending=np.argwhere(closed&(labels<0))
 if not len(pending):break
 for i,j in pending:
  near=[(labels[ii,jj],z0[ii,jj]) for ii,jj in [(i-1,j),(i+1,j),(i,j-1),(i,j+1)] if 0<=ii<len(xs) and 0<=jj<len(ys) and labels[ii,jj]>=0]
  if near:labels[i,j]=collections.Counter(x[0] for x in near).most_common(1)[0][0];z0[i,j]=sum(x[1] for x in near)/len(near)
assert not np.any(closed&(labels<0));domain=closed;boundary=domain&~erode(domain)
if a.revision==3:
 # Match the outside support height instead of retaining tall proxy walls.
 context=[o for o in collection.all_objects if o.type=='MESH' and o not in banks and o.get('source_node') in {'ground'}|{f'building-{i:03d}' for i in range(10)}]
 context_bvh,_=make_bvh(context)
 for i,j in np.argwhere(boundary):
  heights=[]
  for di,dj in [(1,0),(-1,0),(0,1),(0,-1),(1,1),(1,-1),(-1,1),(-1,-1)]:
   ii,jj=i+di,j+dj
   if not (0<=ii<len(xs) and 0<=jj<len(ys)) or domain[ii,jj]:continue
   hit,_,_,_=context_bvh.ray_cast(Vector((xs[ii],ys[jj],5000)),Vector((0,0,-1)),10000)
   if hit is not None:heights.append(hit.z)
  if heights:z0[i,j]=min(heights)
z=z0.copy();fixed=boundary.copy();caps=np.full_like(z,np.inf);root_constraints=[];native=json.loads((R.parent/'baseline/masks/manifest.json').read_text());sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-cosine,sine))
def constrain(objects,mask_id,mask_path,label):
 global caps,z,fixed
 bvh,verts=make_bvh(objects);low=min(p.z for p in verts);root_fixed=0
 for i,x in enumerate(xs):
  if x<min(p.x for p in verts)-step or x>max(p.x for p in verts)+step:continue
  for j,y in enumerate(ys):
   if not domain[i,j]:continue
   hit,_,_,_=bvh.ray_cast(Vector((x,y,-500)),Vector((0,0,1)),2000)
   if hit is not None and hit.z<low+45:z[i,j]=hit.z-.5;fixed[i,j]=True;root_fixed+=1
 row=next(r for r in native['masks'] if r['index']==mask_id);left,top=row['box_top_left'];mask=np.asarray(Image.open(mask_path))>0;count=0
 for py,px in zip(*np.nonzero(mask)):
  x=left+px+.5
  if x<xs[0] or x>xs[-1]:continue
  origin=Vector((x,-(top+py+.5)/sine,0))+ray*5000;hit,_,_,_=bvh.ray_cast(origin,-ray,10000)
  if hit is None:continue
  i=min(len(xs)-2,max(0,int(np.searchsorted(xs,x)-1)));limit=hit.z+(hit.y-ys)*sine/cosine-.5;limit=np.where(ys<=hit.y+step,limit,np.inf);caps[i]=np.minimum(caps[i],limit);caps[i+1]=np.minimum(caps[i+1],limit);count+=1
 root_constraints.append(dict(asset=label,mask=mask_id,domain_sha256=sha(mask_path),native_rays=count,root_grid_constraints=root_fixed))
wood=[o for o in collection.all_objects if o.type=='MESH' and o.get('source_node')==f'scenery-tree{tree_id:02d}-wood'];constrain(wood,tree_id,source.parents[1]/'wood-domain.png',cfg['asset_id'])
neighbors=[(0,'approved-tree00-wood-fill-v1/croisement01-tree-00/baked-v4-support/worker.blend','tree00-v4/wood-domain.png'),(1,'approved-tree01-isolated-wood-fill-v1/croisement01-tree-01/baked-v1-luminance/worker.blend','tree01-source-prep-v1/wood-domain-proposal.png'),(2,'tree02-v8/assets/croisement01-tree-02/model.blend','tree02-v8/wood-domain.png'),(3,'approved-tree03-fill-v1/croisement01-tree-03/baked-v1-luminance/worker.blend','tree03-v4/wood-domain.png')]
if tree_id==5 and a.revision>=4:neighbors.append((6,'tree06-v6/assets/croisement01-tree-06/model.blend','tree06-v6/wood-domain.png'))
for n,path,mask in neighbors:
 path=R/path
 with bpy.data.libraries.load(str(path),link=False) as (src,dst):dst.objects=list(src.objects)
 loaded=[o for o in dst.objects if o is not None]
 for imported in loaded:bpy.context.scene.collection.objects.link(imported)
 bpy.context.view_layer.update()
 targets=[o for o in loaded if o.type=='MESH' and o.get('asset_group')==f'croisement01-tree-{n:02d}' and 'foliage' not in o.get('source_node','') and o.get('projection_component')!='crown']
 assert targets
 reference=next(row for row in json.loads((R/('bank-neighbor-transform-reference-v2.json' if n==6 else 'bank-neighbor-transform-reference-v1.json')).read_text())['sources'] if row['mask']==n);assert reference['model_sha256']==sha(path)
 assert len(reference['objects'])==len(targets)
 for target in targets:
  expected=[row for row in reference['objects'] if row['source_node']==target.get('source_node')];assert len(expected)==1
  assert max(abs(target.matrix_world[i][j]-expected[0]['matrix_world'][i][j]) for i in range(4) for j in range(4))<1e-5,'Neighbor evaluated transform mismatch'
 constrain(targets,n,R/mask,f'croisement01-tree-{n:02d}');root_constraints[-1]['model_sha256']=sha(path)
 for o in loaded:bpy.data.objects.remove(o,do_unlink=True)
violations=int(np.count_nonzero(boundary&(caps<z0-.01)));z=np.minimum(z,caps);z[~domain]=0
for it in range(4000):
 sums=np.zeros_like(z);count=np.zeros_like(z)
 for di,dj in [(1,0),(-1,0),(0,1),(0,-1)]:
  mask=np.roll(domain,(di,dj),(0,1));sums+=np.roll(z,(di,dj),(0,1))*mask;count+=mask
 proposed=np.where(count>0,(sums+.01*z0)/(np.maximum(count,1)+.01),z);proposed[fixed]=z[fixed];proposed=np.minimum(proposed,caps);proposed[~domain]=0;delta=float(np.max(np.abs(proposed-z)));z=proposed
 if delta<1e-5:break
mesh_reports=[]
for label,bank in enumerate(banks):
 cells={(i,j) for i in range(len(xs)-1) for j in range(len(ys)-1) if domain[i:i+2,j:j+2].all() and collections.Counter(labels[i:i+2,j:j+2].ravel()).most_common(1)[0][0]==nodes.index(bank['source_node'])};verts=[];polys=[];components=0
 while cells:
  first=cells.pop();todo=[first];component=[first]
  while todo:
   i,j=todo.pop()
   for other in [(i-1,j),(i+1,j),(i,j-1),(i,j+1)]:
    if other in cells:cells.remove(other);component.append(other);todo.append(other)
  mapping={};top=[];offset=len(verts)
  for i,j in component:
   row=[]
   for k in [(i,j),(i+1,j),(i+1,j+1),(i,j+1)]:
    if k not in mapping:mapping[k]=len(mapping)
    row.append(mapping[k])
   top.append(row)
  ordered=sorted(mapping,key=mapping.get);n=len(ordered);verts.extend((float(xs[i]),float(ys[j]),float(z[i,j])) for i,j in ordered);verts.extend((float(xs[i]),float(ys[j]),-1.) for i,j in ordered);counts={}
  for face in top:
   polys.append(tuple(offset+k for k in face));polys.append(tuple(offset+n+k for k in reversed(face)))
   for x,y in zip(face,face[1:]+face[:1]):key=tuple(sorted((x,y)));counts[key]=(counts.get(key,(0,None))[0]+1,(x,y))
  for uses,(x,y) in counts.values():
   if uses==1:polys.append((offset+x,offset+n+x,offset+n+y,offset+y))
  components+=1
 mesh=bpy.data.meshes.new('Complete inferred bank surface');mesh.from_pydata(verts,[],polys);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bad=sum(not e.is_manifold for e in bm.edges);deg=sum(f.calc_area()<1e-8 for f in bm.faces);bm.to_mesh(mesh);bm.free();inverse=bank.matrix_world.inverted()
 for v in mesh.vertices:v.co=inverse@v.co
 # Supply the same heightfield normal on both owners' shared border.
 # Closed underground side faces must not pull top shading toward vertical.
 normals=[None]*len(mesh.loops)
 for f in mesh.polygons:
  top_face=all(verts[k][2]>-.999 for k in f.vertices);f.use_smooth=top_face
  for loop_index in f.loop_indices:
   vertex_index=mesh.loops[loop_index].vertex_index
   if top_face:
    x,y,_=verts[vertex_index];i=int(round((x-xs[0])/step));j=int(round((y-ys[0])/step))
    il=max(0,i-1);ir=min(len(xs)-1,i+1);jl=max(0,j-1);jr=min(len(ys)-1,j+1)
    if not domain[il,j]:il=i
    if not domain[ir,j]:ir=i
    if not domain[i,jl]:jl=j
    if not domain[i,jr]:jr=j
    dx=(z[ir,j]-z[il,j])/max(step,(ir-il)*step);dy=(z[i,jr]-z[i,jl])/max(step,(jr-jl)*step)
    normals[loop_index]=(bank.matrix_world.to_3x3().transposed()@Vector((-dx,-dy,1))).normalized()
   else:normals[loop_index]=f.normal
 mesh.normals_split_custom_set(normals)
 for mat in bank.data.materials:mesh.materials.append(mat)
 bank.data=mesh;mesh_reports.append(dict(node=bank['source_node'],vertices=len(verts),faces=len(polys),components=components,nonmanifold_edges=bad,degenerate_faces=deg))
assert all(geom(o)==protected[o.get('source_node')] for o in collection.all_objects if o.type=='MESH' and o not in banks)
bpy.data.orphans_purge(do_recursive=True);(worker/'modified').mkdir(parents=True);(worker/'inspection').mkdir();shutil.copy2(source/'modified/views.json',worker/'modified/views.json');(worker/'workspace.json').write_text(json.dumps(cfg,indent=2)+'\n');bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));assert sha(source/'model.blend')==source_hash
(dest/'construction.json').write_text(json.dumps(dict(status='Private complete-bank hypothesis; root and user approval pending',source_sha256=source_hash,model_sha256=sha(worker/'model.blend'),wood_and_unrelated_geometry_unchanged=True,changed_nodes=nodes,grid_step=step,inferred='Smooth complete adjacent bank surfaces within their combined native footprint; no rectangular patch. Original elevations are soft constraints. Root geometry/native visibility constrained, no canonical gameplay changes.',root_constraints=root_constraints,boundary_visibility_conflicts=violations,filled_gap_grid_samples=int(filled.sum()),iterations=it+1,convergence=delta,max_raise=float(np.max((z-z0)[domain])),max_lower=float(np.max((z0-z)[domain])),topology=mesh_reports),indent=2)+'\n');print(worker)
