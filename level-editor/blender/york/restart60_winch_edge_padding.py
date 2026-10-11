"""Repair ten subpixel UV-edge sampling gaps from adjacent generated same-face texels."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';I=W/'restart60-winch-chain-combined-v2';O=W/'restart60-winch-chain-combined-v3';assert not O.exists();sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert shutil.disk_usage(ROOT).free>10*1024**3;assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024>6*1024**3
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire(slots=2)
import bpy,numpy as np
from refinement_workspace import _geometry
bpy.ops.wm.open_mainfile(filepath=str(I/'worker.blend'));scene=bpy.context.scene;audit=json.loads((I/'residual-edge-audit.json').read_text());assert sha(I/'worker.blend')==audit['model_sha256'];assert len(audit['points'])==10
before_geometry={o.name:_geometry(o) for o in scene.objects};names={p['object'] for p in audit['points']};outside={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name not in names};report=json.loads((I/'validation.json').read_text());O.mkdir();(O/'provenance-0').mkdir();repairs=[]
for row in report['layers'][0]['objects']:
 original=Path(row['texel_provenance']['path']);dest=O/'provenance-0'/original.name
 if row['object'] not in names:
  shutil.copy2(original,dest);row['texel_provenance']['path']=str(dest);continue
 o=scene.objects[row['object']];mats={o.data.materials[p.material_index] for p in o.data.polygons};assert len(mats)==1;mat=next(iter(mats));uv=o.data.uv_layers[next(n.uv_map for n in mat.node_tree.nodes if n.type=='UVMAP')];im=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');owner=np.load(original)['ownership'];h,w=owner.shape;buffer=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(buffer);before=buffer.reshape(h,w,4).copy();after=before.copy();initial_owner=owner.copy();o.data.calc_loop_triangles();written=np.zeros((h,w),bool)
 for point in (p for p in audit['points'] if p['object']==o.name):
  assert len(point['matches'])==1;match=point['matches'][0];assert 0<=match['minimum_polygon_edge_distance_texels']<=.25;face=match['face'];domain=np.zeros((h,w),bool)
  for tri in (t for t in o.data.loop_triangles if t.polygon_index==face):
   pts=np.array([list(uv.data[i].uv) for i in tri.loops])*[w,h];lo=np.maximum(0,np.floor(pts.min(axis=0)).astype(int));hi=np.minimum([w,h],np.ceil(pts.max(axis=0)).astype(int));yy,xx=np.mgrid[lo[1]:hi[1],lo[0]:hi[0]];a,b,c=pts;v0=b-a;v1=c-a;q=np.stack([xx+.5,yy+.5],-1)-a;den=v0[0]*v1[1]-v1[0]*v0[1]
   if abs(den)<1e-10:continue
   s=(q[:,:,0]*v1[1]-v1[0]*q[:,:,1])/den;t=(v0[0]*q[:,:,1]-q[:,:,0]*v0[1])/den;domain[lo[1]:hi[1],lo[0]:hi[0]]|=(s>=0)&(t>=0)&(s+t<=1)
  y,x=point['uv_pixel'];assert owner[y,x]==0 and domain[y,x];donors=np.argwhere(domain&(initial_owner==2));assert len(donors);dist=((donors-[y,x])**2).sum(axis=1);dy,dx=donors[int(dist.argmin())];distance=float(np.sqrt(dist.min()));assert distance<=2,(o.name,face,distance)
  after[y,x]=before[dy,dx];owner[y,x]=3;written[y,x]=True;repairs.append({'object':o.name,'face':face,'destination':[y,x],'generated_same_face_donor':[int(dy),int(dx)],'distance_texels':distance,'distance_to_polygon_edge_texels':match['minimum_polygon_edge_distance_texels'],'before_rgba':before[y,x].tolist(),'after_rgba':after[y,x].tolist()})
 assert np.array_equal(before[~written],after[~written]);im.pixels.foreach_set(after.ravel());im.update();im.pack();np.savez_compressed(dest,ownership=owner);row['texel_provenance'].update(path=str(dest),sha256=sha(dest),packed_image_sha256=hashlib.sha256(im.packed_file.data).hexdigest(),rgba8_sha256=hashlib.sha256(np.rint(np.clip(after,0,1)*255).astype(np.uint8).tobytes()).hexdigest())
assert len(repairs)==10;assert before_geometry=={o.name:_geometry(o) for o in scene.objects};assert outside=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in outside};bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(O/'worker.blend'),compress=True)
report['edge_sampling_correction']={'source_model_sha256':sha(I/'worker.blend'),'source_edge_audit_sha256':sha(I/'residual-edge-audit.json'),'scope':'Ten subpixel UV-edge sampling gaps only. Donors are existing generated texels inside the exact same polygon; no source pixels, prior generated pixels, geometry, UVs or outside appearances changed.','max_distance_texels':2,'max_distance_to_edge_texels':.25,'repairs':repairs};(O/'validation.json').write_text(json.dumps(report,indent=2)+'\n');(O/'all-pose-preservation.json').write_text(json.dumps({'status':'GEOMETRY_AND_OUTSIDE_APPEARANCE_EXACT_AT_SAVED_POSE','saved_model_sha256':sha(O/'worker.blend'),'receiver_count':85,'padding_destination_count':10,'independent_all45_guard':'Pending'},indent=2)+'\n');print(O)
