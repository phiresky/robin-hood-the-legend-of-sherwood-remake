"""Alpha-aware native first-hit audit for the private tree and local crown."""
import sys,math,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';RAY=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))));SIN=math.sin(math.radians(35))
def main():
 version=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'v1';out=B/f'tree04-crown-prototype-{version}';assert not (out/'native-audit.json').exists();acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(out/'worker.blend'));scene=bpy.data.scenes['Tree13 isolated wood'];rows=[]
  for obj in [o for o in scene.objects if o.type=='MESH']:
   mesh=obj.data;mesh.calc_loop_triangles();vs=[obj.matrix_world@v.co for v in mesh.vertices];ts=list(mesh.loop_triangles);tree=BVHTree.FromPolygons(vs,[list(t.vertices) for t in ts],all_triangles=True);foliage=obj.get('asset_group')=='croisement03-arbre07-fragment-tree04-provisional';mat=mesh.materials[-1];node=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');uv=mesh.uv_layers['UVMap'];values=np.asarray(node.image.pixels[:],np.float32).reshape(node.image.size[1],node.image.size[0],4);rows.append((obj,tree,vs,ts,uv,values,foliage))
  box=(353,0,479,220);w,h=126,220;expected=np.zeros((h,w,4),np.uint8);bark=np.array(Image.open(B/'tree04-bark-proposal-v1/proposed-bark.png').crop(box))>0;src=np.array(Image.open(B.parent/'baseline/covered.png').convert('RGBA').crop(box));expected[bark]=src[bark];leaf=np.array(Image.open(B/'tree04-canopy-fragment-source-v1/000.png'));expected[:154,10:116][leaf[:,:,3]>0]=leaf[leaf[:,:,3]>0];actual=np.zeros((h,w,4),np.uint8);owner=np.zeros((h,w),np.uint8);exhausted=0
  for yy in range(h):
   for xx in range(w):
    origin=Vector((box[0]+xx+.5,-(yy+.5)/SIN,0))+RAY*10000
    for step in range(256):
     hits=[]
     for i,(o,t,vs,ts,uv,rgba,fol) in enumerate(rows):
      p,n,f,d=t.ray_cast(origin,-RAY)
      if p is not None:hits.append((d,i,p,n,f))
     if not hits:break
     d,i,p,n,f=min(hits,key=lambda r:r[0]);o,t,vs,ts,uv,rgba,fol=rows[i];tri=ts[f];u=barycentric_transform(p,*[vs[j] for j in tri.vertices],*[Vector((*uv.data[j].uv,0)) for j in tri.loops]);inside=0<=u.x<1 and 0<=u.y<1;pixel=rgba[min(rgba.shape[0]-1,int(u.y*rgba.shape[0])),min(rgba.shape[1]-1,int(u.x*rgba.shape[1]))] if inside else np.zeros(4)
     if pixel[3]>.5 and (fol or n.dot(RAY)>0):actual[yy,xx]=np.rint(pixel*255).astype(np.uint8);owner[yy,xx]=2 if fol else 1;break
     if not fol:actual[yy,xx]=[100,100,100,255];owner[yy,xx]=3;break
     origin=p-RAY*.002
    else:exhausted+=1
  known=expected[:,:,3]>0;miss=known&(actual[:,:,3]==0);changed=known&np.any(expected[:,:,:3]!=actual[:,:,:3],axis=2);leafdomain=np.zeros((h,w),bool);leafdomain[:154,10:116]=leaf[:,:,3]>0;visiblebark=bark&~leafdomain;bark_changed=visiblebark&(changed|(owner!=1));leafchanged=leafdomain&(changed|(owner!=2));diff=src.copy();diff[changed]=[255,0,100,255];diff[bark_changed]=[0,200,255,255];sheet=Image.new('RGBA',(w*3,h),(40,40,40,255));sheet.paste(Image.fromarray(expected),(0,0));sheet.paste(Image.fromarray(actual),(w,0));sheet.paste(Image.fromarray(diff),(w*2,0));sheet.resize((w*9,h*3),Image.Resampling.NEAREST).save(out/'native-comparison.png');np.savez_compressed(out/'native-samples.npz',expected=expected,actual=actual,owner=owner)
  write_json(out/'native-audit.json',dict(model_sha256=sha(out/'worker.blend'),status='PASS provisional source first-hit' if not changed.any() and not exhausted else 'HOLD source changes',accepted_bark_pixels=int(bark.sum()),visible_bark_pixels=int(visiblebark.sum()),bark_occluded_by_diagnostic_frame0=int((bark&leafdomain).sum()),accepted_bark_changes=int(bark_changed.sum()),provisional_foliage_pixels=int(leafdomain.sum()),provisional_foliage_changes=int(leafchanged.sum()),known_misses=int(miss.sum()),ray_exhaustions=exhausted,changed_pixels=[[int(xx+box[0]),int(yy)] for yy,xx in zip(*np.where(changed))],limits=['Exact dynamic frame0 interval first-hit does not prove runtime/global ordering or static exclusive membership.','Unknown wood and outside-mask source domains remain unassigned.']))
 finally:release()
if __name__=='__main__':main()
