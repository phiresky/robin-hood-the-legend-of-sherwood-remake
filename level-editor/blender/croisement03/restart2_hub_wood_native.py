"""Verify approved north-tree source rays after mechanical wood normalization."""
import sys,math,json,shutil
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
def main(number):
 assert number in range(6,12)
 config={6:((535,0,620,220),154,10,75,'07'),7:((600,0,735,280),154,10,130,'07'),8:((736,0,792,190),135,6,39,'06'),9:((769,0,811,180),135,6,31,'06'),10:((794,0,856,195),135,6,56,'06'),11:((844,0,952,195),135,6,102,'06')}
 box,leafheight,leafleft,leafright,sprite=config[number];asset=f'croisement03-tree-{number:02}';leafgroup=f'croisement03-arbre{sprite}-fragment-tree{number:02}-provisional'
 assert shutil.disk_usage(ROOT).free>10*1024**3;available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024;assert available>=6*1024**3
 out=B/f'approved-hub-textures-v1/{asset}/wood-input-v1';assert not (out/'native-audit.json').exists();norm=json.loads((out/'normalization.json').read_text());assert sha(out/'normalized.blend')==norm['normalized_model_sha256'];assert sha(norm['approved_model'])==norm['approved_model_sha256'];acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(out/'normalized.blend'));scene=bpy.data.scenes['Croisement03 Refinement'];rows=[]
  for obj in [o for o in scene.objects if o.type=='MESH']:
   mesh=obj.data;mesh.calc_loop_triangles();vs=[obj.matrix_world@v.co for v in mesh.vertices];ts=list(mesh.loop_triangles);tree=BVHTree.FromPolygons(vs,[list(t.vertices) for t in ts],all_triangles=True);foliage=obj.get('asset_group')==leafgroup;mat=mesh.materials[-1];node=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');uv=mesh.uv_layers['UVMap'];values=np.asarray(node.image.pixels[:],np.float32).reshape(node.image.size[1],node.image.size[0],4);rows.append((obj,tree,vs,ts,uv,values,foliage))
  w,h=box[2]-box[0],box[3]-box[1];expected=np.zeros((h,w,4),np.uint8);bark=np.array(Image.open(json.loads((out/'prepared.json').read_text())['bark_domain']).crop(box))>0;src=np.array(Image.open(B.parent/'baseline/covered.png').convert('RGBA').crop(box));expected[bark]=src[bark];leaf=np.array(Image.open(B/f'tree{number:02}-canopy-fragment-source-v1/000.png'));expected[:leafheight,leafleft:leafright][leaf[:,:,3]>0]=leaf[leaf[:,:,3]>0];actual=np.zeros((h,w,4),np.uint8);owner=np.zeros((h,w),np.uint8);exhausted=0
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
  known=expected[:,:,3]>0;miss=known&(actual[:,:,3]==0);changed=known&np.any(expected[:,:,:3]!=actual[:,:,:3],axis=2);leafdomain=np.zeros((h,w),bool);leafdomain[:leafheight,leafleft:leafright]=leaf[:,:,3]>0;visiblebark=bark&~leafdomain;bark_changed=visiblebark&(changed|(owner!=1));leafchanged=leafdomain&(changed|(owner!=2));diff=src.copy();diff[changed]=[255,0,100,255];diff[bark_changed]=[0,200,255,255];sheet=Image.new('RGBA',(w*3,h),(40,40,40,255));sheet.paste(Image.fromarray(expected),(0,0));sheet.paste(Image.fromarray(actual),(w,0));sheet.paste(Image.fromarray(diff),(w*2,0));sheet.resize((w*9,h*3),Image.Resampling.NEAREST).save(out/'native-comparison.png');np.savez_compressed(out/'native-samples.npz',expected=expected,actual=actual,owner=owner)
  write_json(out/'native-audit.json',dict(model_sha256=sha(out/'normalized.blend'),status='PASS provisional source first-hit' if not changed.any() and not exhausted else 'HOLD source changes',accepted_bark_pixels=int(bark.sum()),visible_bark_pixels=int(visiblebark.sum()),bark_occluded_by_diagnostic_frame0=int((bark&leafdomain).sum()),accepted_bark_changes=int(bark_changed.sum()),provisional_foliage_pixels=int(leafdomain.sum()),provisional_foliage_changes=int(leafchanged.sum()),known_misses=int(miss.sum()),ray_exhaustions=exhausted,changed_pixels=[[int(xx+box[0]),int(yy)] for yy,xx in zip(*np.where(changed))],limits=['Exact dynamic frame0 interval first-hit does not prove runtime/global ordering or static exclusive membership.','Unknown wood and outside-mask source domains remain unassigned.']))
 finally:release()
if __name__=='__main__':
 args=sys.argv[sys.argv.index('--')+1:];assert len(args)==1;main(int(args[0]))
