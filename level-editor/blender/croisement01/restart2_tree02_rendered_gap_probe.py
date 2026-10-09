"""Trace rendered coverage witnesses to exact atlas texels and physical leaves."""
import hashlib,json,shutil,sys,collections
from pathlib import Path
import bpy,numpy as np
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
residual='--residual' in sys.argv
R=ROOT/'level-editor/work/croisement01-refinement/restart2';B=R/'approved-tree02-fill-v1/croisement01-tree-02/baked-v3-same-leaf-crown';B=B.with_name('baked-v4-bounded-bark-gaps') if residual else B;O=R/('tree02-filtered-gap-probe-v3' if residual else 'tree02-rendered-gap-probe-v2' if '--extended' in sys.argv else 'tree02-rendered-gap-probe-v1');assert not O.exists();assert shutil.disk_usage(R).free>=10*1024**3+1024**2;assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2;acquire();O.mkdir();sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();digest=sha(B/'worker.blend');assert digest==('3210379a9da2409e63fe5eba4c349260168dd611040c5077514367718130e9bd' if residual else 'cc794f3b3fb91b825b2d247f8ad116b7d8fe301d31107c353c0d8f386f050877');bpy.ops.wm.open_mainfile(filepath=str(B/'worker.blend'));manifest=json.loads((B/'coverage-views-384.json').read_text());validation=json.loads((B/'validation.json').read_text());names=manifest['object_names'];vertices=[];faces=[];owners=[];objects={};atlas={};uvs={};rgba={};packets={}
scene=bpy.data.scenes[manifest['scene_name']];bpy.context.window.scene=scene;bpy.context.view_layer.update();assert scene.render.pixel_aspect_x==scene.render.pixel_aspect_y
for e in validation['layers'][0]['objects']:
 o=bpy.data.objects[e['object']];objects[o.name]=o;assert not any(mod.show_render for mod in o.modifiers), 'Evaluated geometry needs an explicit probe';m=o.data;m.calc_loop_triangles();base=len(vertices);vertices.extend(o.matrix_world@v.co for v in m.vertices)
 used={f.material_index for f in m.polygons};materials=[mat for i,mat in enumerate(m.materials) if i in used and mat and mat.use_nodes];uvnames={n.uv_map for mat in materials for n in mat.node_tree.nodes if n.type=='UVMAP' and n.uv_map};assert len(uvnames)==1;uvs[o.name]=m.uv_layers[next(iter(uvnames))].data;atlas[o.name]=np.load(e['texel_provenance']['path'])['ownership'];h,w=atlas[o.name].shape;images={n.image.name:n.image for mat in materials for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image and tuple(n.image.size)==(w,h)};assert len(images)==1;im=next(iter(images.values()));p=np.empty(w*h*4,np.float32);im.pixels.foreach_get(p);rgba[o.name]=p.reshape(h,w,4)
 for tri in m.loop_triangles:faces.append(tuple(base+i for i in tri.vertices));owners.append((o.name,tri.index,tri.polygon_index))
tree=BVHTree.FromPolygons(vertices,faces,all_triangles=True);witnesses=[];target_faces=collections.defaultdict(set)
for view_index,px,py in ([(0,148,94),(0,149,94),(0,149,95),(1,192,155),(3,218,79)] if residual else [(0,149,94),(0,149,95),(1,192,155),(1,192,164),(3,218,79)]):
 view=next(v for v in manifest['views'] if v['index']==view_index);matrix=Matrix(view['camera_matrix_world']);scale=view['ortho_scale'];direction=matrix.to_3x3()@Vector((0,0,-1));hits=[]
 offsets=np.linspace(-1,2,49).tolist() if residual else [-.375,-.125,.125,.375,.5,.625,.875,1.125,1.375] if '--extended' in sys.argv else [.125,.375,.5,.625,.875]
 for ox in offsets:
  for oy in offsets:
   origin=matrix@Vector((((px+ox)/384-.5)*scale,(.5-(py+oy)/384)*scale,0));hit=tree.ray_cast(origin,direction)
   if hit[2] is None:continue
   name,ti,fi=owners[hit[2]];o=objects[name];tri=o.data.loop_triangles[ti];points=np.array([(o.matrix_world@o.data.vertices[i].co)[:] for i in tri.vertices]);weights=np.linalg.lstsq(np.vstack([points.T,np.ones(3)]),np.r_[np.array(hit[0]),1],rcond=None)[0];uv=np.array([uvs[name][i].uv[:] for i in tri.loops]);tex=weights@uv;h,w=atlas[name].shape;x,y=np.clip((tex*[w,h]).astype(int),[0,0],[w-1,h-1]);ownership=int(atlas[name][y,x]);row={'subpixel':[ox,oy],'object':name,'face':fi,'triangle':ti,'atlas_texel':[int(x),int(y)],'ownership':ownership,'weights':weights.tolist()};hits.append(row)
   if ownership==0:target_faces[name].add(fi)
 if residual:
  unique={}
  for h in hits:
   key=(h['object'],h['face'],h['triangle'],tuple(h['atlas_texel']))
   if key not in unique:unique[key]=dict(h,samples=0)
   unique[key]['samples']+=1
  hits=list(unique.values())
 witnesses.append({'view':view_index,'pixel':[px,py],'hits':hits,'unfilled_hits':sum(x['ownership']==0 for x in hits)})
for name,selected in target_faces.items():
 o=objects[name];m=o.data;original_selected=sorted(selected);adjacency=[]
 if name=='Tree02 inferred crown':
  parent=list(range(len(m.vertices)))
  def find(v):
   while parent[v]!=v:parent[v]=parent[parent[v]];v=parent[v]
   return v
  for f in m.polygons:
   for v in f.vertices[1:]:parent[find(v)]=find(f.vertices[0])
  components=collections.defaultdict(list)
  for f in m.polygons:components[find(f.vertices[0])].append(f.index)
  groups=[g for g in components.values() if set(g)&selected];assert all(len(g)==2 for g in groups);selected={f for g in groups for f in g}
 else:
  if '--extended' in sys.argv or residual:
   edge_faces=collections.defaultdict(list)
   for face in m.polygons:
    for a,b in zip(face.vertices,list(face.vertices[1:])+[face.vertices[0]]):edge_faces[tuple(sorted((int(a),int(b))))].append(face.index)
   for edge,fs in edge_faces.items():
    if set(fs)&set(original_selected):selected.update(fs);adjacency.append({'vertices':list(edge),'faces':fs})
  groups=[[f] for f in sorted(selected)]
 ts=[];verts=set();h,w=atlas[name].shape
 for t in m.loop_triangles:
  if t.polygon_index in selected:ts.append({'face':t.polygon_index,'vertices':list(t.vertices),'uv':[list(uvs[name][i].uv) for i in t.loops]});verts.update(t.vertices)
 # Exact local tile values suffice; avoid copying the whole model or atlas.
 coords={}
 for t in ts:
  q=np.array(t['uv'])*[w,h];lo=np.maximum(0,np.floor(q.min(0)-1).astype(int));hi=np.minimum([w-1,h-1],np.ceil(q.max(0)+1).astype(int))
  for y in range(lo[1],hi[1]+1):
   for x in range(lo[0],hi[0]+1):coords[f'{x},{y}']={'ownership':int(atlas[name][y,x]),'rgba':rgba[name][y,x].tolist()}
 packets[name]={'witness_faces':original_selected,'adjacency':adjacency,'groups':groups,'atlas_size':[w,h],'triangles':ts,'vertices_world':{str(v):list(o.matrix_world@m.vertices[v].co) for v in verts},'texels':coords}
report={'model_sha256':digest,'witnesses':witnesses,'packets':packets,'render_settings':{'pixel_aspect':[scene.render.pixel_aspect_x,scene.render.pixel_aspect_y],'engine':scene.render.engine,'filter_type':getattr(scene.cycles,'pixel_filter_type',None),'filter_width':getattr(scene.cycles,'filter_width',None),'modifiers':'asserted absent for render','depsgraph_updated':True,'sample_spacing':.0625 if residual else None},'scope':'Orthographic subpixel first opaque geometry hits. Extended mode includes a bounded pixel filter neighborhood and exact shared-edge donor context; no repairs or mask relaxation.'};(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');assert sha(B/'worker.blend')==digest;size=sum(p.stat().st_size for p in O.iterdir());assert size<=(4 if residual else 1)*1024**2;release();print(json.dumps({'bytes':size,'witnesses':[{k:v for k,v in x.items() if k!='hits'} for x in witnesses],'objects':{k:v['groups'] for k,v in packets.items()}}),flush=True)
