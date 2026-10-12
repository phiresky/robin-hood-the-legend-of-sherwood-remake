"""Validate reviewed, all-phase-clear bark seeds against unchanged saved wood receivers."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
sys.path.insert(0,str(Path(__file__).parent))
import restart2_tree01_canopy_saved as common
B=common.B;RAY=Vector((0,-common.C,common.S));sha=common.sha

def main():
 folder=B/'tree01-native-transition-saved-v2';model=folder/'worker.blend';out=folder/'bark-receiver-294-v1'
 maskpath=B/'tree01-role-exclusions-v1/all-phase-clear-bark-proposal.png';count=294
 if '--proposal' in sys.argv:maskpath=Path(sys.argv[sys.argv.index('--proposal')+1]).resolve()
 if '--out' in sys.argv:out=Path(sys.argv[sys.argv.index('--out')+1]).resolve()
 if '--count' in sys.argv:count=int(sys.argv[sys.argv.index('--count')+1]);assert 0<count<=2000
 assert not out.exists()
 common.resources();common.acquire(slots=2)
 try:
  common.resources();out.mkdir();digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));rows=[]
  for o in bpy.context.scene.objects:
   wood=o.name.startswith('Provisional Tree01 wood');crown=o.name.startswith('Tree01 ')
   if o.type!='MESH' or not(wood or crown):continue
   m=o.data;m.calc_loop_triangles();v=[o.matrix_world@p.co for p in m.vertices];tris=list(m.loop_triangles);row={'name':o.name,'wood':wood,'tree':BVHTree.FromPolygons(v,[list(t.vertices)for t in tris],all_triangles=True),'v':v,'tris':tris,'context':o.get('source_obstacle_context')}
   if crown:
    image=next(n.image for n in m.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');row.update(uv=m.uv_layers.active,pixels=np.asarray(image.pixels[:],np.float32).reshape(image.size[1],image.size[0],4))
   rows.append(row)
  def hit(row,origin):
   probe=origin.copy()
   for step in range(512):
    p,n,f,d=row['tree'].ray_cast(probe,-RAY)
    if p is None:return None
    if row['wood']:return ((p-origin).length,f,list(p))
    if n.dot(RAY)>0:
     t=row['tris'][f];uv=barycentric_transform(p,*[row['v'][j]for j in t.vertices],*[Vector((*row['uv'].data[j].uv,0))for j in t.loops]);a=row['pixels']
     if 0<=uv.x<=1 and 0<=uv.y<=1 and a[min(int(uv.y*a.shape[0]),a.shape[0]-1),min(int(uv.x*a.shape[1]),a.shape[1]-1),3]>.5:return((p-origin).length,f,list(p))
    probe=p-RAY*.002
   raise AssertionError('Alpha traversal exhausted')
  mask=np.array(Image.open(maskpath))>0;assert int(mask.sum())==count
  phasepath=B/'tree01-bark-completeness-v2/receipt.json';phaseproof=json.loads(phasepath.read_text());native_overlap=set()
  for frame in phaseproof['full_animation_source']['frames']:
   path=Path(frame['path']);assert sha(path)==frame['sha256'];a=np.array(Image.open(path).convert('RGBA'));x0,y0=frame['bbox'][:2]
   for y,x in np.argwhere(mask):
    xx,yy=int(x)-x0,int(y)-y0
    if 0<=xx<a.shape[1]and 0<=yy<a.shape[0]and a[yy,xx,3]:native_overlap.add((int(x),int(y)))
  source=B.parent/'baseline/covered.png';rgba=np.array(Image.open(source).convert('RGBA'));fail=[];samples=[]
  upper=json.loads((B/'tree01-upper-bark-proposal-v1/proposal.json').read_text());expected={tuple(xy):r['proposed_mesh_owner']for r in upper['rows']for xy in r['candidate_pixels']}
  for y,x in np.argwhere(mask):
   origin=Vector((float(x)+.5,-(float(y)+.5)/common.S,0))+RAY*5000;hits=[]
   for row in rows:
    result=hit(row,origin)
    if result is not None:hits.append((result[0],row,result))
   first=min(hits,key=lambda h:h[0])if hits else None
   sample={'source':[int(x),int(y)],'source_rgba':rgba[y,x].tolist(),'expected_upper_owner':expected.get((int(x),int(y)))}
   if first:
    _,row,result=first;sample.update(receiver=row['name'],context=row['context'],triangle=result[1],world=result[2]);wrong=not row['wood'] or(sample['expected_upper_owner']is not None and str(sample['expected_upper_owner'])not in row['name'].split())
   else:wrong=True;sample['receiver']=None
   if wrong:fail.append(sample)
   samples.append(sample)
  report={'status':'PASS_SAVED_BARK_RECEIVERS'if not fail and not native_overlap else'HOLD_SAVED_BARK_RECEIVERS','model_sha256':digest,'source_sha256':sha(source),'mask_sha256':sha(maskpath),'full_native_phase_proof_sha256':sha(phasepath),'full_native_overlap':sorted(native_overlap),'samples':samples,'failures':fail,'objects':[{'name':r['name'],'context':r['context']}for r in rows],'limits':['Read-only saved geometry check; no texture assignment or geometry approval.','All15 native-leaf-crossing old samples and seven upper CPU misses remain excluded.','Static proposal cells retained diagnostically; their source ownership is unresolved.']};(out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');assert sha(model)==digest;print({'status':report['status'],'samples':len(samples),'failures':len(fail),'full_native_overlap':len(native_overlap)});assert not fail and not native_overlap
 finally:common.release()
if __name__=='__main__':main()
