"""Reopen and review the bounded tree39 contour derivative."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector,Matrix
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_tree39_contour import ROOT,OUT,SPECS,RAY,SIN,COS,covered
from evidence_io import sha,write_json
from render_slots import acquire,release
from bake_texture_candidate import snapshot,pixels,array_hash
from refinement_review import _tree
from restart4_stump_final_contact import frame,sheet
from render_multiview_asset import render
def own():return [o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-39']
def main():
 out=ROOT/'tree39-contour-v6';model=out/'model.blend';digest=sha(model);source,oldhash=SPECS[39];assert sha(source)==oldhash;inv=json.loads((OUT/'review-mask-inventory.json').read_text());m=next(x for x in inv['masks']if x['index']==39);native_mask=np.array(Image.open(m['png']))>0;oy,ox=m['box_top_left'][1],m['box_top_left'][0];ys,xs=np.where(native_mask);targets=np.column_stack((xs+ox+.5,ys+oy+.5));bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();objects=own();old_positions={o.name:np.array([o.matrix_world@v.co for v in o.data.vertices])for o in objects};old_uv={o.name:{u.name:array_hash(np.array([v.uv[:]for v in u.data]))for u in o.data.uv_layers}for o in objects};oldcov=covered([o for o in objects if 'Crown'not in o.name],targets);old_tri={o.name:[tuple(p.vertices)for p in o.data.polygons]for o in objects};oldimages={im.name:hashlib.sha256(bytes(im.packed_file.data)).hexdigest()for o in objects for mat in o.data.materials if mat and mat.use_nodes for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and (im:=n.image)and im.packed_file}
 bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();scene=bpy.context.scene;objects=own();wood=[o for o in objects if 'Crown'not in o.name];newcov=covered(wood,targets);lost=targets[oldcov&~newcov]-.5;newimages={im.name:hashlib.sha256(bytes(im.packed_file.data)).hexdigest()for o in objects for mat in o.data.materials if mat and mat.use_nodes for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and (im:=n.image)and im.packed_file};assert all(newimages.get(n)==h for n,h in oldimages.items());moved=[]
 for o in objects:
  assert [tuple(p.vertices)for p in o.data.polygons]==old_tri[o.name]
  assert all(array_hash(np.array([v.uv[:]for v in o.data.uv_layers[n].data]))==h for n,h in old_uv[o.name].items());p=np.array([o.matrix_world@v.co for v in o.data.vertices]);delta=np.linalg.norm(p-old_positions[o.name],axis=1);moved.extend(p[delta>1e-5]);q=np.column_stack((old_positions[o.name][:,0],-old_positions[o.name][:,1]*SIN-old_positions[o.name][:,2]*COS));assert np.all(delta[q[:,1]<=565]==0)
  if 'Crown'in o.name or o.get('source_node')=='building-097':assert np.all(delta==0)
 mask=np.array(Image.open(ROOT/'source-audit-v1/exposed-39.png'))>0;yy,xx=np.where(mask);tree,owners,_=_tree(wood);tris=[]
 for o in wood:o.data.calc_loop_triangles();tris.extend((o,t)for t in o.data.loop_triangles)
 native=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));samples=[];failures=[];cache={}
 for y,x in zip(yy,xx):
  p,n,i,d=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);assert p is not None;o,t=tris[i];mat=o.data.materials[t.material_index];node=next((n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image and n.image.name.startswith('native39')),None)
  if not node:failures.append(dict(pixel=[int(x),int(y)],reason='No exact native overlay at saved first hit',object=o.name,material=mat.name));continue
  uv=o.data.uv_layers['Exact lower contour native projection'];q=barycentric_transform(p,*[o.matrix_world@o.data.vertices[j].co for j in t.vertices],*[Vector((*uv.data[j].uv,0))for j in t.loops]);a=cache.get(node.image.name)
  if a is None:a=np.rint(pixels(node.image)*255).astype('uint8');cache[node.image.name]=a
  h,w=a.shape[:2];value=a[min(h-1,max(0,int(q.y*h))),min(w-1,max(0,int(q.x*w)))];ok=np.array_equal(value,native[y,x]);samples.append(dict(pixel=[int(x),int(y)],exact=bool(ok),z=p.z))
  if not ok:failures.append(dict(pixel=[int(x),int(y)],reason='Native RGBA mismatch',actual=value.tolist(),expected=native[y,x].tolist()))
 write_json(out/'saved-guard.json',dict(model_sha256=digest,source_sha256=oldhash,original_topology_uv_and_packed_images_exact=True,upper_support_crown_wood097_exact=True,all_native_mask_pixels=len(targets),old_native_coverage=int(oldcov.sum()),new_native_coverage=int(newcov.sum()),lost_native_coverage=lost.tolist(),target_count=226,target_native_exact=sum(x['exact']for x in samples),failures=failures))
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';base=json.loads((source.parent.parent/'views.json').read_text());base['object_names']=[o.name for o in objects];base.pop('render_object_names',None)
 for i,v in enumerate(base['views']):
  direction=RAY if i==0 else Matrix(v['camera_matrix_world']).to_quaternion()@Vector((0,0,1));cam=frame(scene,objects,direction,384,1.22);v['camera_matrix_world']=[list(r)for r in cam.matrix_world];v['ortho_scale']=cam.data.ortho_scale;v['crop']={'width':384,'height':384}
 write_json(out/'cameras.json',base);render(out/'cameras.json',out/'actual',modes=('textured','solid'),width=384)
 for mode in ['textured','solid']:sheet([out/f'actual/view-{i}-{mode}.png'for i in range(8)],out/f'{mode}-sheet.png')
 for o in objects:o.hide_render='Crown'in o.name
 caminfo=json.loads((ROOT/'baseline-audit-39-v2/camera.json').read_text());camera=bpy.data.objects.new('Original native close',bpy.data.cameras.new('Original native close'));scene.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.ortho_scale=caminfo['scale'];camera.data.clip_end=20000;camera.matrix_world=Matrix(caminfo['camera']);scene.camera=camera;scene.render.resolution_x=512;scene.render.resolution_y=512;scene.render.resolution_percentage=100;scene.render.filepath=str(out/'native-wood.png');bpy.ops.render.render(write_still=True);orig=Image.open(ROOT/'baseline-audit-39-v2/native-source.png').convert('RGBA');pic=Image.open(out/'native-wood.png').convert('RGBA');comparison=Image.new('RGBA',(1536,512),(35,35,35,255));comparison.paste(orig,(0,0));comparison.paste(Image.alpha_composite(orig,pic),(512,0));comparison.paste(pic,(1024,0));comparison.save(out/'source-comparison.png')
 mesh=bpy.data.meshes.new('Changed lower contour framing points');mesh.from_pydata(moved,[],[]);proxy=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(proxy);proxy.hide_render=True;center=np.array(moved).mean(0);bpy.ops.mesh.primitive_plane_add(size=220,location=(center[0],center[1],0));floor=bpy.context.object;floor.name='Neutral Z0 contact guide';mat=bpy.data.materials.new('Neutral contact guide');mat.diffuse_color=(.12,.13,.10,1);floor.data.materials.append(mat);directions=[RAY,Vector((.6124,-.6124,.5)),Vector((.6124,.6124,.5)),Vector((-.866,0,.5))]
 for i,d in enumerate(directions):frame(scene,[proxy],d.normalized(),512,1.25);scene.render.filepath=str(out/f'contact-{i}.png');bpy.ops.render.render(write_still=True)
 sheet([out/f'contact-{i}.png'for i in range(4)],out/'contact-sheet.png');assert sha(model)==digest;print('SAVED GUARD',len(lost),'lost',len(failures),'native failures',flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
