"""Read-only attribution of reverse leaf ribbons to mapping and overlapping layers."""
import sys,re,json,hashlib,collections
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement')]
from render_slots import acquire,release

def main():
 e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment';out=e/'ribbon-layer-diagnostic-v1';assert not out.exists();model=e/'native-rgb-control-v1/worker.blend';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();assert sha(model)=='58d09f01e0cde42e7ca9b46d0a2f958d388bba23cf4b35f09a6d7bd1aec81802';acquire();bpy.ops.wm.open_mainfile(filepath=str(model));obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25');mesh=obj.data;mesh.calc_loop_triangles();vertices=[obj.matrix_world@v.co for v in mesh.vertices];tris=list(mesh.loop_triangles);bvh=BVHTree.FromPolygons(vertices,[list(t.vertices) for t in tris],all_triangles=True);atlas={}
 for slot,mat in enumerate(mesh.materials):
  if not mat or not mat.get('foliage_physical_opacity'):continue
  node=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);a=np.empty(len(node.image.pixels),np.float32);node.image.pixels.foreach_get(a);flags={mesh.color_attributes['Source ownership'].data[i].color[0] for f in mesh.polygons if f.material_index==slot for i in f.loop_indices};atlas[slot]=dict(alpha=a.reshape(node.image.size[1],node.image.size[0],4)[...,3],uv=mesh.uv_layers[node.inputs['Vector'].links[0].from_node.uv_map],name=mat.name,lobe=int(re.search(r'lobe(\d+)',mat.name)[1]),kind='transverse' if 'transverse' in mat.name else ('source-shell' if flags=={1.} else 'unknown-shell'))
 def trace(origin,direction):
  hits=[]
  for step in range(256):
   p,n,tid,d=bvh.ray_cast(origin,direction)
   if p is None:return hits,False
   tri=tris[tid];slot=mesh.polygons[tri.polygon_index].material_index
   if slot not in atlas:hits.append((slot,tid,'wood'));return hits,False
   data=atlas[slot];uv=data['uv'];mapped=barycentric_transform(p,*[vertices[v] for v in tri.vertices],*[Vector((*uv.data[i].uv,0)) for i in tri.loops]);a=data['alpha'];x=min(a.shape[1]-1,int((mapped.x%1)*a.shape[1]));y=min(a.shape[0]-1,int((mapped.y%1)*a.shape[0]))
   if a[y,x]>=.5:hits.append((slot,tid,data['kind']))
   origin=p+direction*1e-4
  return hits,True
 manifest=json.loads((e/'views.json').read_text());regions=[(0,(85,95,185,195)),(4,(120,160,280,320)),(5,(80,120,280,320)),(1,(100,95,285,300))];out.mkdir();reports=[]
 palette=[(210,70,50),(230,140,40),(230,220,40),(80,210,60),(40,180,180),(70,100,240),(170,80,230),(230,100,170),(150,120,80)]
 for index,box in regions:
  view=manifest['views'][index];matrix=Matrix(view['camera_matrix_world']);inverse=matrix.inverted();direction=matrix.to_3x3()@Vector((0,0,-1));direction.normalize();scale=view['ortho_scale'];counts=collections.Counter();by_slot=collections.Counter();ratios=collections.defaultdict(list);unique_lobes=collections.Counter();coverage_loss=collections.Counter();cache={};mask=Image.new('RGB',(384,384),(25,25,25));draw=ImageDraw.Draw(mask)
  for y in range(box[1],box[3],2):
   for x in range(box[0],box[2],2):
    hits,limited=trace(matrix@Vector((((x+.5)/384-.5)*scale,(.5-(y+.5)/384)*scale,0)),direction);counts['sampled']+=1;counts['depth_limit']+=int(limited)
    if not hits:counts['transparent']+=1;continue
    counts['opaque']+=1;slot,tid,kind=hits[0];by_slot[slot]+=1;lobes={atlas[s]['lobe'] for s,t,k in hits if s in atlas};unique_lobes[len(lobes)]+=1
    if not any(k!='transverse' for s,t,k in hits):coverage_loss['remove_all_transverse']+=1
    for lobe in range(9):
     if not any(s not in atlas or atlas[s]['lobe']!=lobe for s,t,k in hits):coverage_loss[f'remove_lobe{lobe:02}']+=1
    if slot not in atlas:draw.rectangle((x,y,x+1,y+1),fill=(180,180,180));continue
    draw.rectangle((x,y,x+1,y+1),fill=palette[atlas[slot]['lobe']])
    if tid not in cache:
     tri=tris[tid];data=atlas[slot];p=np.array([inverse@vertices[v] for v in tri.vertices]);uv=np.array([data['uv'].data[i].uv[:] for i in tri.loops])*[data['alpha'].shape[1],data['alpha'].shape[0]];u=np.stack([uv[1]-uv[0],uv[2]-uv[0]],axis=1)
     if abs(np.linalg.det(u))<1e-10:cache[tid]=None
     else:
      j=np.stack([p[1,:2]-p[0,:2],p[2,:2]-p[0,:2]],axis=1)@np.linalg.inv(u)*384/scale;sv=np.linalg.svd(j,compute_uv=False);cache[tid]=float(sv[0]/max(sv[1],1e-12))
    if cache[tid] is not None:ratios[slot].append(cache[tid])
  mask.save(out/f'view-{index}-first-lobe.png');rows=[]
  for slot,count in by_slot.most_common():
   d=atlas.get(slot);vals=ratios.get(slot,[]);rows.append(dict(slot=slot,material=d['name'] if d else 'wood',lobe=d['lobe'] if d else None,kind=d['kind'] if d else 'wood',first_visible_samples=count,screen_uv_anisotropy_median=float(np.median(vals)) if vals else None,screen_uv_anisotropy_p95=float(np.quantile(vals,.95)) if vals else None))
  reports.append(dict(view=index,box=box,pixel_step=2,counts=dict(counts),first_visible=rows,distinct_opaque_lobes_per_ray=dict(unique_lobes),hypothetical_opaque_samples_lost=dict(coverage_loss)));print('Finished view',index,flush=True)
 receipt=dict(model_sha256=sha(model),reports=reports,legend={str(i):c for i,c in enumerate(palette)},method='Pixel-centre nearest-alpha rays every2pixels in declared regions. Collect all physical opaque hits; first-hit lobe identifies visible ribbons. Screen-UV Jacobian ratios are per-triangle, weighted by first-hit samples. Hypothetical removals only count geometric coverage without changing the model.',limitations=['Diagnostic sample, not full-frame antialias or final render proof.','Screen anisotropy includes camera foreshortening; compare independent world-UV audit.','No source, geometry, UV, alpha or material changed.']);(out/'evidence.json').write_text(json.dumps(receipt,indent=2)+'\n');assert sha(model)==receipt['model_sha256'];release()
if __name__=='__main__':main()
