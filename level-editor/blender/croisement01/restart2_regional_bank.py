"""Private regional soil transition, retaining clipped exterior background surfaces."""
import argparse,collections,hashlib,json,math,shutil,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
p=argparse.ArgumentParser();p.add_argument('--tree',type=int,choices=[4,5],required=True);p.add_argument('--revision',type=int,required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);R=ROOT/'level-editor/work/croisement01-refinement/restart2';tree_id=a.tree;source_revision={4:6,5:7}[tree_id];source=R/f'tree{tree_id:02d}-v{source_revision}/assets/croisement01-tree-{tree_id:02d}';dest=R/f'tree{tree_id:02d}-regional-banks-v{a.revision}';worker=dest/f'assets/croisement01-tree-{tree_id:02d}';assert not dest.exists();acquire();bpy.ops.wm.open_mainfile(filepath=str(source/'model.blend'));bpy.context.preferences.filepaths.save_version=0;cfg=json.loads((source/'workspace.json').read_text());collection=bpy.data.collections[cfg['collection_name']];nodes=([f'building-{i:03d}' for i in range(2,6)] if tree_id==4 and a.revision>=4 else {4:['building-003','building-004'],5:['building-006','building-007']}[tree_id]);assert tree_id==5;nodes=['building-000','building-006','building-007'];banks=[o for o in collection.all_objects if o.type=='MESH' and o.get('source_node') in nodes];source_hash=sha(source/'model.blend')
def geom(o):return hashlib.sha256(json.dumps([[list(v.co) for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons]],separators=(',',':')).encode()).hexdigest()
def make_bvh(objects):
 points=[];faces=[]
 for o in objects:
  off=len(points);points.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(off+k for k in f.vertices) for f in o.data.polygons)
 return BVHTree.FromPolygons(points,faces),points
# The bounded background splice retains its original planar faces outside this
# rectangle. New intersections lie on those original surfaces; no exterior
# original vertex moves. Terrain inside is explicitly inferred.
region=(300.,640.,-980.,-540.)
def split_polygon(poly,axis,bound,keep_low):
 inside=[];outside=[]
 for current,following in zip(poly,poly[1:]+poly[:1]):
  a=current[axis]-bound;b=following[axis]-bound;in_a=a<=0 if keep_low else a>=0;in_b=b<=0 if keep_low else b>=0
  (inside if in_a else outside).append(current)
  if in_a!=in_b:
   point=current+(following-current)*(a/(a-b));inside.append(point);outside.append(point)
 return inside,outside
def clip_region(poly):
 inside=poly;outside=[]
 for axis,bound,keep_low in [(0,region[0],False),(0,region[1],True),(1,region[2],False),(1,region[3],True)]:
  if not inside:break
  inside,part=split_polygon(inside,axis,bound,keep_low)
  if len(part)>=3:outside.append(part)
 return inside,outside
preserved_background=[];background_original_faces=[];preserved_materials=[];exterior_schema=[]
protected={o.get('source_node'):geom(o) for o in collection.all_objects if o.type=='MESH'};points=[];faces=[];owners=[]
for o in banks:
 off=len(points);points.extend(o.matrix_world@v.co for v in o.data.vertices)
 if o.get('source_node')=='building-000':
  exterior_schema=[('uv',layer.name,2) for layer in o.data.uv_layers]+[('color',layer.name,4) for layer in o.data.color_attributes if layer.domain=='CORNER']
 for f in o.data.polygons:
  normal=(o.matrix_world.to_3x3().inverted().transposed()@f.normal).normalized()
  poly=[o.matrix_world@o.data.vertices[k].co for k in f.vertices]
  if o.get('source_node')=='building-000':
   background_original_faces.append([list(v) for v in poly]);payload=[]
   for point,loop_index in zip(poly,f.loop_indices):
    values=list(point)
    for kind,name,width in exterior_schema:values.extend(o.data.uv_layers[name].data[loop_index].uv if kind=='uv' else o.data.color_attributes[name].data[loop_index].color)
    payload.append(np.asarray(values,dtype=float))
   clipped,exterior=clip_region(payload);poly=[Vector(row[:3]) for row in clipped];preserved_background.extend(exterior);preserved_materials.extend([f.material_index]*len(exterior))
  if normal.z>.1 and len(poly)>=3:
   start=len(points);points.extend(poly);faces.append(tuple(range(start,len(points))));owners.append(o.get('source_node'))
assert faces;top_tree=BVHTree.FromPolygons(points,faces);step=4.;padding=64.;xs=np.arange(math.floor(min(p.x for p in points)/step)*step-padding,math.ceil(max(p.x for p in points)/step)*step+padding+step*.1,step);ys=np.arange(math.floor(min(p.y for p in points)/step)*step-padding,math.ceil(max(p.y for p in points)/step)*step+padding+step*.1,step);z0=np.zeros((len(xs),len(ys)));domain=np.zeros_like(z0,dtype=bool);labels=np.full_like(z0,-1,dtype=int)
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
# Fill the region as one broad hillside, not a narrow root-following ramp.
regional=(xs[:,None]>=region[0])&(xs[:,None]<=region[1])&(ys[None,:]>=region[2])&(ys[None,:]<=region[3])
extension=regional&~closed;closed|=regional
background=[o for o in collection.all_objects if o.type=='MESH' and o.get('source_node') in {'building-000','ground'}];background_tree,_=make_bvh(background)
for i,j in np.argwhere(extension):
 hit,_,_,_=background_tree.ray_cast(Vector((xs[i],ys[j],5000)),Vector((0,0,-1)),10000);assert hit is not None
 z0[i,j]=hit.z;labels[i,j]=nodes.index('building-000')
closed[[0,-1]]=False;closed[:,[0,-1]]=False
filled=closed&~domain
for _ in range(max(len(xs),len(ys))):
 pending=np.argwhere(closed&(labels<0))
 if not len(pending):break
 for i,j in pending:
  near=[(labels[ii,jj],z0[ii,jj]) for ii,jj in [(i-1,j),(i+1,j),(i,j-1),(i,j+1)] if 0<=ii<len(xs) and 0<=jj<len(ys) and labels[ii,jj]>=0]
  if near:labels[i,j]=collections.Counter(x[0] for x in near).most_common(1)[0][0];z0[i,j]=sum(x[1] for x in near)/len(near)
assert not np.any(closed&(labels<0));domain=closed;boundary=domain&~erode(domain)
z=z0.copy();fixed=boundary.copy();caps=np.full_like(z,np.inf);root_constraints=[];native=json.loads((R.parent/'baseline/masks/manifest.json').read_text());sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-cosine,sine))
def constrain(objects,mask_id,mask_path,label):
 global caps,z,fixed
 bvh,verts=make_bvh(objects);low=min(p.z for p in verts);root_fixed=0;root_samples=[];root_plane=None
 for i,x in enumerate(xs):
  if x<min(p.x for p in verts)-step or x>max(p.x for p in verts)+step:continue
  for j,y in enumerate(ys):
   if not domain[i,j]:continue
   hit,_,_,_=bvh.ray_cast(Vector((x,y,-500)),Vector((0,0,1)),2000)
   if hit is not None and hit.z<low+45:
    root_samples.append((i,j,float(hit.z)));z[i,j]=hit.z+(8. if mask_id==6 and a.revision>=3 else -.5);fixed[i,j]=True;root_fixed+=1
 if mask_id==6 and a.revision>=4:
  # Embed the hidden basal volume beneath one gentle slope. Do not trace
  # every underside vertex into the ground or reveal a closed root end cap.
  sy=np.asarray([ys[j] for i,j,h in root_samples]);front=np.quantile(sy,.1);back=np.quantile(sy,.75)
  front_rows=[(ys[j],h) for i,j,h in root_samples if ys[j]<=front];back_rows=[(ys[j],h) for i,j,h in root_samples if ys[j]>=back]
  yf=float(np.mean([y for y,h in front_rows]));yb=float(np.mean([y for y,h in back_rows]));zf=float(np.median([h for y,h in front_rows]))+.5;zb=float(np.quantile([h for y,h in back_rows],.25))+8.;slope=(zb-zf)/(yb-yf)
  for i,j,h in root_samples:z[i,j]=zf+(ys[j]-yf)*slope
  root_plane=dict(front=[yf,zf],back=[yb,zb],dz_dy=slope,status='Inferred ground slope anchored to front root and embedded rear base; native visibility caps remain authoritative')
 row=next(r for r in native['masks'] if r['index']==mask_id);left,top=row['box_top_left'];mask=np.asarray(Image.open(mask_path))>0;count=0
 for py,px in zip(*np.nonzero(mask)):
  x=left+px+.5
  if x<xs[0] or x>xs[-1]:continue
  origin=Vector((x,-(top+py+.5)/sine,0))+ray*5000;hit,_,_,_=bvh.ray_cast(origin,-ray,10000)
  if hit is None:continue
  i=min(len(xs)-2,max(0,int(np.searchsorted(xs,x)-1)));limit=hit.z+(hit.y-ys)*sine/cosine-.5;limit=np.where(ys<=hit.y+step,limit,np.inf);caps[i]=np.minimum(caps[i],limit);caps[i+1]=np.minimum(caps[i+1],limit);count+=1
 root_constraints.append(dict(asset=label,mask=mask_id,domain_sha256=sha(mask_path),native_rays=count,root_grid_constraints=root_fixed,inferred_root_embedding=(8. if mask_id==6 and a.revision>=3 else -.5),root_plane=root_plane))
wood=[o for o in collection.all_objects if o.type=='MESH' and o.get('source_node')==f'scenery-tree{tree_id:02d}-wood'];constrain(wood,tree_id,source.parents[1]/'wood-domain.png',cfg['asset_id'])
neighbors=[(0,'approved-tree00-wood-fill-v1/croisement01-tree-00/baked-v4-support/worker.blend','tree00-v4/wood-domain.png'),(1,'approved-tree01-isolated-wood-fill-v1/croisement01-tree-01/baked-v1-luminance/worker.blend','tree01-source-prep-v1/wood-domain-proposal.png'),(2,'tree02-v8/assets/croisement01-tree-02/model.blend','tree02-v8/wood-domain.png'),(3,'approved-tree03-fill-v1/croisement01-tree-03/baked-v1-luminance/worker.blend','tree03-v4/wood-domain.png')]
root_revision=9 if a.revision>=2 else 8
neighbors.extend([(4,'tree04-v6/assets/croisement01-tree-04/model.blend','tree04-v6/wood-domain.png'),(6,f'tree06-v{root_revision}/assets/croisement01-tree-06/model.blend',f'tree06-v{root_revision}/wood-domain.png')])
for n,path,mask in neighbors:
 path=R/path
 with bpy.data.libraries.load(str(path),link=False) as (src,dst):dst.objects=list(src.objects)
 loaded=[o for o in dst.objects if o is not None]
 for imported in loaded:bpy.context.scene.collection.objects.link(imported)
 bpy.context.view_layer.update()
 targets=[o for o in loaded if o.type=='MESH' and o.get('asset_group')==f'croisement01-tree-{n:02d}' and 'foliage' not in o.get('source_node','') and o.get('projection_component')!='crown']
 assert targets
 reference=next(row for row in json.loads((R/f'bank-neighbor-transform-reference-tree06-v{root_revision}.json').read_text())['sources'] if row['mask']==n);assert reference['model_sha256']==sha(path)
 assert len(reference['objects'])==len(targets)
 for target in targets:
  expected=[row for row in reference['objects'] if row['source_node']==target.get('source_node')];assert len(expected)==1
  assert max(abs(target.matrix_world[i][j]-expected[0]['matrix_world'][i][j]) for i in range(4) for j in range(4))<1e-5,'Neighbor evaluated transform mismatch'
 constrain(targets,n,R/mask,f'croisement01-tree-{n:02d}');root_constraints[-1]['model_sha256']=sha(path);root_constraints[-1]['model_path']=str(path);root_constraints[-1]['domain_path']=str(R/mask)
 for o in loaded:bpy.data.objects.remove(o,do_unlink=True)
violations=int(np.count_nonzero(boundary&(caps<z0-.01)));z=np.minimum(z,caps);z[~domain]=0
for it in range(4000):
 sums=np.zeros_like(z);count=np.zeros_like(z)
 for di,dj in [(1,0),(-1,0),(0,1),(0,-1)]:
  mask=np.roll(domain,(di,dj),(0,1));sums+=np.roll(z,(di,dj),(0,1))*mask;count+=mask
 proposed=np.where(count>0,(sums+.001*z0)/(np.maximum(count,1)+.001),z);proposed[fixed]=z[fixed];proposed=np.minimum(proposed,caps);proposed[~domain]=0;delta=float(np.max(np.abs(proposed-z)));z=proposed
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
 exterior_face_start=len(polys)
 if bank['source_node']=='building-000':
  for poly in preserved_background:
   offset=len(verts);verts.extend(tuple(v[:3]) for v in poly);polys.append(tuple(range(offset,len(verts))))
 mesh=bpy.data.meshes.new('Complete inferred bank surface');mesh.from_pydata(verts,[],polys);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bad=sum(not e.is_manifold for e in bm.edges);deg=sum(f.calc_area()<1e-8 for f in bm.faces);bm.to_mesh(mesh);bm.free();inverse=bank.matrix_world.inverted()
 for v in mesh.vertices:v.co=inverse@v.co
 # Supply the same heightfield normal on both owners' shared border.
 # Closed underground side faces must not pull top shading toward vertical.
 normals=[None]*len(mesh.loops)
 for f in mesh.polygons:
  top_face=all(verts[k][2]>-.999 for k in f.vertices) and all(abs((verts[k][0]-xs[0])/step-round((verts[k][0]-xs[0])/step))<1e-5 and abs((verts[k][1]-ys[0])/step-round((verts[k][1]-ys[0])/step))<1e-5 for k in f.vertices);f.use_smooth=top_face
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
 if bank['source_node']=='building-000':
  for kind,name,width in exterior_schema:
   if kind=='uv':mesh.uv_layers.new(name=name)
   else:mesh.color_attributes.new(name=name,type='FLOAT_COLOR',domain='CORNER')
  # bmesh preserved face order here; explicitly bind each exterior loop to its
  # clipped original coordinate before copying its interpolated corner data.
  for index,(payload,material_index) in enumerate(zip(preserved_background,preserved_materials)):
   face=mesh.polygons[exterior_face_start+index];face.material_index=material_index
   for loop_index in face.loop_indices:
    world=bank.matrix_world@mesh.vertices[mesh.loops[loop_index].vertex_index].co
    matches=[row for row in payload if (Vector(row[:3])-world).length<.001];assert matches
    row=matches[0];cursor=3
    for kind,name,width in exterior_schema:
     if kind=='uv':mesh.uv_layers[name].data[loop_index].uv=row[cursor:cursor+width]
     else:mesh.color_attributes[name].data[loop_index].color=row[cursor:cursor+width]
     cursor+=width
 bank.data=mesh;mesh_reports.append(dict(node=bank['source_node'],vertices=len(verts),faces=len(polys),components=components,nonmanifold_edges=bad,degenerate_faces=deg))
assert all(geom(o)==protected[o.get('source_node')] for o in collection.all_objects if o.type=='MESH' and o not in banks)
bpy.data.orphans_purge(do_recursive=True);(worker/'modified').mkdir(parents=True);(worker/'inspection').mkdir();shutil.copy2(source/'modified/views.json',worker/'modified/views.json');(worker/'workspace.json').write_text(json.dumps(cfg,indent=2)+'\n');bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));assert sha(source/'model.blend')==source_hash
(dest/'construction.json').write_text(json.dumps(dict(status='Private complete-bank hypothesis; root and user approval pending',source_sha256=source_hash,model_sha256=sha(worker/'model.blend'),wood_and_unrelated_geometry_unchanged=True,changed_nodes=nodes,grid_step=step,inferred='Broad regional slope for banks006/007 plus bounded background000 splice. Exterior000 planar geometry retained by exact clipping. Tree06 lowered along native camera rays; original proxy heights are weak priors, not observations. Unknown soil appearance, seams and topology require review.',root_constraints=root_constraints,background_region=list(region),preserved_background_polygons=len(preserved_background),preserved_exterior_corner_schema=exterior_schema,preserved_background_surface_sha256=hashlib.sha256(json.dumps([[list(v) for v in poly] for poly in preserved_background],separators=(',',':')).encode()).hexdigest(),boundary_visibility_conflicts=violations,filled_gap_grid_samples=int(filled.sum()),rounded_root_shoulder_grid_samples=int(extension.sum()),iterations=it+1,convergence=delta,max_raise=float(np.max((z-z0)[domain])),max_lower=float(np.max((z0-z)[domain])),topology=mesh_reports),indent=2)+'\n');print(worker)
