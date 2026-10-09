"""Propagate only inferred mound appearance, restoring each site's source ownership."""
import sys,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from bake_texture_candidate import snapshot,pixels
from refinement_review import _tree
from tree_geometry import RAY,SIN
BASE=OUT/'restart25-approved-state-materialization-v1'
def main():
 assert shutil.disk_usage(OUT).free>=10*1024**3
 assert int(next(l.split()[1]for l in Path('/proc/meminfo').read_text().splitlines()if l.startswith('MemAvailable:')))*1024>=6*1024**3
 authority_path=BASE/'mound-ownership-v1/report.json';assert sha(authority_path)=='28b8161f8daac462486cdb7dab3b8310e58d5b1d287fd9c192d3616334b4d46f';authority=json.loads(authority_path.read_text());source_model=Path(authority['approval']['member']['model']);assert sha(source_model)==authority['approval']['member']['model_sha256'];arr=authority['ownership'];assert sha(Path(arr['path']))==arr['sha256'];owned=np.load(arr['path']);source=Path(authority['source']['path']);assert sha(source)==authority['source']['sha256'];native=np.array(Image.open(source).convert('RGBA'));h,w=native.shape[:2]
 template=BASE/'official-texture-experiments-v2/mound-initial/experiment/native-state-v4/worker.blend';proof=json.loads(template.with_name('preservation.json').read_text());assert sha(template)==proof['model_sha256'];out=BASE/'mound-filled-all-sites-v1';out.mkdir(exist_ok=False)
 bpy.ops.wm.open_mainfile(filepath=str(source_model));scene=bpy.context.scene;names={r['object']for site in authority['records']for r in site['objects']};before=snapshot(scene,names);targets={n:scene.objects[n] for n in names};native_uv='Native source projection';records={n:dict(slots=[p.material_index for p in o.data.polygons],uv=np.array([d.uv[:]for d in o.data.uv_layers[native_uv].data]))for n,o in targets.items()}
 tempnames=[r['object']for r in authority['records'][0]['objects']]
 with bpy.data.libraries.load(str(template),link=False)as(src,dst):dst.objects=tempnames
 templates=dict(zip(tempnames,dst.objects));assert len(templates)==67
 for site in authority['records']:
  for index,row in enumerate(site['objects']):
   o=targets[row['object']];t=templates[tempnames[index]];assert len(o.data.vertices)==len(t.data.vertices) and [list(p.vertices)for p in o.data.polygons]==[list(p.vertices)for p in t.data.polygons];assert np.array_equal(records[o.name]['uv'],np.array([d.uv[:]for d in t.data.uv_layers[native_uv].data]))
   for uv in t.data.uv_layers:
    if uv.name==native_uv:continue
    assert o.data.uv_layers.get(uv.name)is None;layer=o.data.uv_layers.new(name=uv.name)
    for dest,src in zip(layer.data,uv.data):dest.uv=src.uv
   original_known=next(m for m in o.data.materials if m.name==row['observed_material']);native_tex=next(n for n in original_known.node_tree.nodes if n.type=='TEX_IMAGE');known_template=next(m for m in t.data.materials if 'exact native over inferred fill' in m.name);composite=known_template.copy();composite.name=o.name+' exact native with inferred fill'
   for n in composite.node_tree.nodes:
    if n.type!='TEX_IMAGE' or not n.image or not n.inputs['Vector'].links:continue
    uvname=n.inputs['Vector'].links[0].from_node.uv_map
    if uvname!=native_uv:continue
    if n.image.colorspace_settings.name!='Non-Color':n.image=native_tex.image
    else:
     mask=out/(o.name.replace(' ','_')+'-ownership.png');rgba=np.zeros((h,w,4),np.uint8);rgba[:,:,:3]=np.where(owned[row['ownership_array']][:,:,None],255,0);rgba[:,:,3]=255;Image.fromarray(rgba).save(mask);n.image=bpy.data.images.load(str(mask));n.image.colorspace_settings.name='Non-Color';n.image.pack()
   generated=next(m for m in t.data.materials if m and m.get('source_ownership_bake'));o.data.materials.append(composite);knownslot=len(o.data.materials)-1;o.data.materials.append(generated);unknownslot=len(o.data.materials)-1
   for p,old in zip(o.data.polygons,records[o.name]['slots']):p.material_index=knownslot if o.data.materials[old]==original_known else unknownslot
  print('Filled',site['site'],flush=True)
 assert snapshot(scene,names)==before
 for t in templates.values():bpy.data.objects.remove(t,do_unlink=True)
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True);digest=sha(out/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(out/'worker.blend'));scene=bpy.context.scene;bpy.context.view_layer.update();assert snapshot(scene,names)==before;results=[]
 for site in authority['records']:
  objects=[scene.objects[r['object']] for r in site['objects']];rows={r['object']:r for r in site['objects']};tris=[];vertices=[];faces=[]
  for o in objects:
   assert not o.modifiers
   o.data.calc_loop_triangles();offset=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(offset+i for i in t.vertices)for t in o.data.loop_triangles);tris.extend((o,t)for t in o.data.loop_triangles)
  tree=BVHTree.FromPolygons(vertices,faces,all_triangles=True)
  origin=site['source_origin'];tested=0;failures=[]
  for y,x in np.argwhere(native[:,:,3]>0):
   p,n,i,d=tree.ray_cast(Vector((origin[0]+x+.5,-(origin[1]+y+.5)/SIN,0))+RAY*2000,-RAY)
   if p is None:failures.append([int(x),int(y),'miss']);continue
   o,t=tris[i];mat=o.data.materials[t.material_index];texs=[nd for nd in mat.node_tree.nodes if nd.type=='TEX_IMAGE' and nd.image and nd.inputs['Vector'].links and nd.inputs['Vector'].links[0].from_node.uv_map==native_uv and nd.image.colorspace_settings.name!='Non-Color'];assert len(texs)==1;q=barycentric_transform(p,*[o.matrix_world@o.data.vertices[j].co for j in t.vertices],*[Vector((*o.data.uv_layers[native_uv].data[j].uv,0))for j in t.loops]);xx=int(q.x*w);yy=int((1-q.y)*h);assert 0<=xx<w and 0<=yy<h
   rgba=np.rint(pixels(texs[0].image)*255).astype(np.uint8)[h-1-yy,xx];ok=np.array_equal(rgba,native[y,x]) and owned[rows[o.name]['ownership_array']][yy,xx];tested+=1
   if not ok:failures.append([int(x),int(y),'native mismatch'])
  assert not failures, (site['site'],failures[:20]);results.append(dict(site=site['site'],native_pixel_centers=tested,failures=failures));print('Verified',site['site'],tested,flush=True)
 write_json(out/'preservation.json',dict(status='PASS',model_sha256=digest,source_model=str(source_model),source_model_sha256=sha(source_model),template_model=str(template),template_model_sha256=sha(template),geometry_uv_unchanged=True,source_ownership_sha256=sha(authority_path),sites=results,texture_approval='pending',actual_material_contact_review='pending',canonical_changed=False))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
