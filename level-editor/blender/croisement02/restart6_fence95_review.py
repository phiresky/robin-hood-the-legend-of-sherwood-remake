"""Reopen exact fence candidate and check retained parts/native post foot."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector,Matrix
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_fence95_contour import ROOT,OUT,SPECS,RAY,SIN,COS,face_digest,covered
from bake_texture_candidate import pixels
from refinement_review import _tree
from evidence_io import sha,write_json
from render_slots import acquire,release
from restart4_stump_final_contact import frame,sheet
from render_views import render_views
ASSET='croisement02-east-upright-rail-fence-95'
def own():return [o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==ASSET]
def main():
 out=ROOT/'fence95-contour-v1';model=out/'model.blend';source,digest=SPECS[95];bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();objs=own();main=bpy.context.scene.objects['East upright rail fence 95'];ids=next(c['vertices']for c in json.loads((ROOT/'components-95-v2.json').read_text())['components']if c['component']==14);outside=face_digest(main,ids);cap=next(o for o in objs if o!=main);capdigest=face_digest(cap);mainname=main.name;capname=cap.name;item=next(x for x in json.loads((OUT/'review-mask-inventory.json').read_text())['masks']if x['index']==95);ox,oy=item['box_top_left'];yy,xx=np.where(np.array(Image.open(item['png']))>0);nativepts=np.column_stack((xx+ox+.5,yy+oy+.5));oldcov=covered(objs,nativepts)
 bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();scene=bpy.context.scene;objs=own();assert face_digest(scene.objects[mainname])==outside;assert face_digest(scene.objects[capname])==capdigest;cov=covered(objs,nativepts);mask=np.array(Image.open(ROOT/'source-audit-v1/exposed-95.png'))>0;yy,xx=np.where(mask);tree,owners,_=_tree(objs);tris=[]
 for o in objs:o.data.calc_loop_triangles();tris.extend((o,t)for t in o.data.loop_triangles)
 native=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));fail=[];samples=[];cache={}
 for y,x in zip(yy,xx):
  hit,n,i,d=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY)
  if hit is None:fail.append([int(x),int(y),'geometry']);continue
  o,t=tris[i];mat=o.data.materials[t.material_index];node=next((n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image and n.image.name.startswith('native95')),None)
  if node is None:fail.append([int(x),int(y),'native material']);continue
  uv=o.data.uv_layers['Exact lower post native projection'];q=barycentric_transform(hit,*[o.matrix_world@o.data.vertices[j].co for j in t.vertices],*[Vector((*uv.data[j].uv,0))for j in t.loops]);a=cache.setdefault(node.image.name,np.rint(pixels(node.image)*255).astype('uint8'));h,w=a.shape[:2];v=a[min(h-1,max(0,int(q.y*h))),min(w-1,max(0,int(q.x*w)))];ok=np.array_equal(v,native[y,x]);samples.append(dict(pixel=[int(x),int(y)],exact=bool(ok),z=hit.z))
  if not ok:fail.append([int(x),int(y),'native RGB'])
 write_json(out/'saved-guard.json',dict(model_sha256=sha(model),source_sha256=digest,all_other_fence_geometry_uv_exact=True,old_native_coverage=int(oldcov.sum()),new_native_coverage=int(cov.sum()),lost=int((oldcov&~cov).sum()),requested=39,exact_native=sum(r['exact']for r in samples),failures=fail))
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.film_transparent=True;scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.render.resolution_x=384;scene.render.resolution_y=384;views={};directions=[RAY,Vector((.6124,-.6124,.5)),Vector((.866,0,.5)),Vector((.6124,.6124,.5)),Vector((0,.866,.5)),Vector((-.6124,.6124,.5)),Vector((-.866,0,.5)),Vector((-.6124,-.6124,.5))]
 for i,d in enumerate(directions):views[f'view-{i}']=frame(scene,objs,d.normalized(),384,1.2).name
 render_views(scene.name,views,out/'actual',modes=('textured','solid'),width=384)
 for mode in ['textured','solid']:sheet([out/f'actual/view-{i}-{mode}.png'for i in range(8)],out/f'{mode}-sheet.png')
 info=json.loads((ROOT/'baseline-audit-95-v3/camera.json').read_text());cam=bpy.data.objects.new('Explicit native post',bpy.data.cameras.new('Explicit native post'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.ortho_scale=info['scale'];cam.data.clip_end=20000;cam.matrix_world=Matrix(info['camera']);render_views(scene.name,{'native':cam.name},out/'native',modes=('textured',),width=512);orig=Image.open(ROOT/'baseline-audit-95-v3/native-source.png').convert('RGBA');pic=Image.open(out/'native/native-textured.png').convert('RGBA');comparison=Image.new('RGBA',(1536,512),(35,35,35,255));comparison.paste(orig,(0,0));comparison.paste(Image.alpha_composite(orig,pic),(512,0));comparison.paste(pic,(1024,0));comparison.save(out/'source-comparison.png')
 post=scene.objects['Source-supported second-post lower contour'];center=sum((post.matrix_world@v.co for v in post.data.vertices),Vector())/len(post.data.vertices);bpy.ops.mesh.primitive_plane_add(size=80,location=(center.x,center.y,0));floor=bpy.context.object;floor.name='Neutral Z0 guide';mat=bpy.data.materials.new('Contact guide');mat.diffuse_color=(.12,.13,.1,1);floor.data.materials.append(mat);views={}
 for i,d in enumerate([directions[0],directions[1],directions[3],directions[6]]):views[f'contact-{i}']=frame(scene,[post],d.normalized(),384,1.45).name
 render_views(scene.name,views,out/'contact',modes=('textured',),width=384);sheet([out/f'contact/contact-{i}-textured.png'for i in range(4)],out/'contact-sheet.png')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
