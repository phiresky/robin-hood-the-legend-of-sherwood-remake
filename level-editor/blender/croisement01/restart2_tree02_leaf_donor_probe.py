"""Extract exact same-leaf atlas ownership without modifying the saved worker."""
import collections,hashlib,json,shutil,sys
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
R=ROOT/'level-editor/work/croisement01-refinement/restart2';B=R/'approved-tree02-fill-v1/croisement01-tree-02/baked-v2-two-sided-crown';O=R/'tree02-leaf-donor-probe-v1'
assert not O.exists();assert shutil.disk_usage(R).free>=10*1024**3+2*1024**2
assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
acquire();O.mkdir();p=B/'worker.blend';sha=lambda q:hashlib.sha256(q.read_bytes()).hexdigest();digest=sha(p);assert digest=='bcf4ffac8bdd81eae32873b4ab26d4d932ed34addcfbf06caad182d4107ad42d';bpy.ops.wm.open_mainfile(filepath=str(p))
o=bpy.data.objects['Tree02 inferred crown'];m=o.data;m.calc_loop_triangles();names={n.uv_map for i,mat in enumerate(m.materials) if i in {f.material_index for f in m.polygons} and mat and mat.use_nodes for n in mat.node_tree.nodes if n.type=='UVMAP' and n.uv_map};assert len(names)==1;uv=m.uv_layers[next(iter(names))].data;entry=next(e for e in json.loads((B/'validation.json').read_text())['layers'][0]['objects'] if e['object']==o.name);ownership=np.load(entry['texel_provenance']['path'])['ownership'];h,w=ownership.shape;triangles=[]
for t in m.loop_triangles:triangles.append({'face':t.polygon_index,'vertices':list(t.vertices),'uv':[[float(x) for x in uv[i].uv] for i in t.loops]})
# Shared vertex connectivity identifies the original physical leaf, without using face-number adjacency as authority.
parent=list(range(len(m.vertices)))
def find(x):
 while parent[x]!=x:parent[x]=parent[parent[x]];x=parent[x]
 return x
for f in m.polygons:
 for v in f.vertices[1:]:parent[find(v)]=find(f.vertices[0])
components=collections.defaultdict(list)
for f in m.polygons:components[find(f.vertices[0])].append(f.index)
leaves=[v for v in components.values()];assert len(leaves)==1800 and all(len(v)==2 for v in leaves)
images={n.image.name:n.image for i,mat in enumerate(m.materials) if i in {f.material_index for f in m.polygons} and mat and mat.use_nodes for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image and tuple(n.image.size)==(w,h)};assert len(images)==1;im=next(iter(images.values()));pixels=np.empty(w*h*4,dtype=np.float32);im.pixels.foreach_get(pixels);rgba=pixels.reshape(h,w,4);packed={'ownership':ownership}
for c in range(4):
 values,index=np.unique(rgba[:,:,c],return_inverse=True);assert len(values)<=256;packed['values_'+str(c)]=values;packed['index_'+str(c)]=index.reshape(h,w).astype(np.uint8);assert np.array_equal(values[packed['index_'+str(c)]],rgba[:,:,c])
np.savez_compressed(O/'atlas.npz',**packed)
targets={1073,1549,1602,1603,1892,1897,2011,2458,2459,2794,2941,3176};leaves=[l for l in leaves if set(l)&targets];faces={f for l in leaves for f in l};triangles=[t for t in triangles if t['face'] in faces];vertices={v for t in triangles for v in t['vertices']}
(O/'mesh.json').write_text(json.dumps({'model_sha256':digest,'object':o.name,'atlas_size':[w,h],'uv_name':next(iter(names)),'triangles':triangles,'leaves':leaves,'full_leaf_count':1800,'vertices_world':{str(v):[float(x) for x in o.matrix_world@m.vertices[v].co] for v in vertices}},separators=(',',':'))+'\n');assert sha(p)==digest;size=sum(x.stat().st_size for x in O.rglob('*') if x.is_file());assert size<=2*1024**2;print('READ ONLY LEAF PROBE COMPLETE',size,flush=True)
