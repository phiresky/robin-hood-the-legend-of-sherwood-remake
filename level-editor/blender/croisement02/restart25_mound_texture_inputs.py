"""Prepare canonical approved mound textures with verified per-clump ownership."""
import sys,json,shutil,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_review import render_review
from render_multiview_asset import render
from restart2_prepare_endpoint_inputs import signature
from prepare_private_texture_inputs import prepare
def main(key):
 assert key=='site-00'
 assert shutil.disk_usage(OUT).free>10*1024**3
 memory=int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024
 assert memory>=6*1024**3
 approval=OUT/'restart3-review-batches/pending-v17-v23-plus-two-hub-v1/user-approval.json';assert sha(approval)=='ca25ba9362b26dfb8ac1239f7acd7b56929463498b125bed0f42dcd98ec628f4'
 user=json.loads(approval.read_text());asset='croisement02-hiding-mounds-all-initial-geometry';member=next(m for c in user['decisions_by_card'] for m in c['members'] if m['asset_id']==asset)
 model=Path(member['model']);base=model.parent;assert sha(model)==member['model_sha256']
 authority_path=OUT/'restart25-approved-state-materialization-v1/mound-ownership-v1/report.json';assert sha(authority_path)=='28b8161f8daac462486cdb7dab3b8310e58d5b1d287fd9c192d3616334b4d46f';authority=json.loads(authority_path.read_text());assert authority['approval']['member']['model_sha256']==member['model_sha256'];site=next(r for r in authority['records'] if r['site']==key);source=Path(authority['source']['path']);assert sha(source)==authority['source']['sha256'];arrays=authority['ownership'];assert sha(Path(arrays['path']))==arrays['sha256'];owned=np.load(arrays['path']);ownership={r['object']:r for r in site['objects']}
 dest=OUT/'restart25-approved-state-materialization-v1/mound-texture-inputs-v1'/key;dest.mkdir(parents=True,exist_ok=False)
 bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();objects=sorted([o for o in scene.objects if o.name in ownership],key=lambda o:o.name);assert len(objects)==67;before=signature(objects);col=bpy.data.collections.new('Mound texture Working');scene.collection.children.link(col)
 canvas=Image.new('RGBA',Image.open(OUT/'baseline/covered.png').size);xy=site['source_origin'];canvas.alpha_composite(Image.open(source).convert('RGBA'),tuple(xy));canvas.save(dest/'source.png');inventory=[];assignments=[]
 for obj in list(bpy.data.objects):
  if obj.type=='MESH' and obj not in objects:bpy.data.objects.remove(obj,do_unlink=True)
 for i,o in enumerate(objects):
  col.objects.link(o);o.hide_render=False;o['asset_group']=asset;o['source_node']=asset+f'-part-{i}';mask=Image.new('L',canvas.size);mask.paste(Image.open(source).convert('RGBA').getchannel('A'),tuple(xy))
  mask.save(dest/f'role-{i}.png');inventory.append(dict(index=i,box_top_left=[0,0],box_size=list(canvas.size),png=f'role-{i}.png'));assignments.append(dict(reviewed=True,source_node=o['source_node'],mask_indices=[i]))
 write_json(dest/'mask-inventory.json',dict(masks=inventory));write_json(dest/'source-masks.json',dict(version=1,mask_inventory='mask-inventory.json',projections=dict(exterior=dict(source_sha256=sha(dest/'source.png'),state='Exact approved mound source; known classification uses verified material and assigned texel',assignments=assignments))))
 assert signature(objects)==before;bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True)
 render_review(dest/'modified',scene_name=scene.name,collection_name=col.name,asset_id=asset,source_path=dest/'source.png',source_mask_manifest=dest/'source-masks.json',width=384,height=384,framing_padding=1.2)
 meta=json.loads((dest/'modified/views.json').read_text());meta['texture_material_suffix']='mound-unknown-v1'
 for v in meta['views']:v['crop']={'left':0,'top':0,'width':384,'height':384}
 write_json(dest/'modified/views.json',meta);scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;render(dest/'modified/views.json',dest/'stored',width=384)
 points=[];triangles=[];records=[];native=np.asarray(Image.open(source).convert('RGBA'));ih,iw=native.shape[:2];uvname=objects[0].data.uv_layers.active.name
 for o in objects:
  o.data.calc_loop_triangles();offset=len(points);points.extend(o.matrix_world@v.co for v in o.data.vertices)
  for t in o.data.loop_triangles:
   triangles.append([offset+i for i in t.vertices]);mat=o.data.materials[t.material_index];known=mat.name==ownership[o.name]['observed_material'];assert known or mat.name==ownership[o.name]['unknown_material'];records.append((known,[o.matrix_world@o.data.vertices[i].co for i in t.vertices],[Vector((*o.data.uv_layers[uvname].data[i].uv,0))for i in t.loops] if known else None,owned[ownership[o.name]['ownership_array']]))
 tree=BVHTree.FromPolygons(points,triangles,all_triangles=True);sheet=Image.new('RGBA',(1536,768));rows=[]
 for v in meta['views']:
  idx=v['index'];m=Matrix(v['camera_matrix_world']);unit=v['ortho_scale']/384;editable=np.zeros((384,384),bool)
  for y in range(384):
   for x in range(384):
    p,n,f,d=tree.ray_cast(m.translation+m.col[0].to_3d()*((x+.5-192)*unit)+m.col[1].to_3d()*((192-y-.5)*unit),-m.col[2].to_3d())
    if p is None:continue
    known,world,uv,assigned=records[f];observed=False
    if known:
     q=barycentric_transform(p,*world,*uv);tx=math.floor(q.x*iw);ty=math.floor((1-q.y)*ih);observed=0<=tx<iw and 0<=ty<ih and assigned[ty,tx]
    editable[y,x]=not observed
  actual=np.array(Image.open(dest/'stored'/f'view-{idx}-textured.png').convert('RGBA'));guide=actual.copy();guide[editable,:3]=77;known=np.full((384,384,4),255,np.uint8);known[editable,:3]=0
  Image.fromarray(guide).save(dest/'modified/views'/f'view-{idx}-textured.png');Image.fromarray(known).save(dest/'modified/views'/f'view-{idx}-known.png');v['ownership_sha256']=sha(dest/'modified/views'/f'view-{idx}-known.png');sheet.paste(Image.fromarray(guide),((idx%4)*384,(idx//4)*384));rows.append(dict(view=idx,editable_pixels=int(editable.sum()),protected_rgba_exact=bool(np.array_equal(actual[~editable],guide[~editable]))))
 sheet.save(dest/'modified/textured.png');write_json(dest/'modified/views.json',meta);assert signature(objects)==before;assert sha(model)==member['model_sha256']
 write_json(dest/'derivation.json',dict(source_model=str(model),source_model_sha256=sha(model),prepared_model_sha256=sha(dest/'model.blend'),geometry_uv_material_signature=before,geometry_uv_materials_unchanged=True,source_image=str(source),source_sha256=sha(source),user_approval=str(approval),user_approval_sha256=sha(approval),object_names=[o.name for o in objects],native_uv=uvname,ownership_authority=str(authority_path),ownership_sha256=sha(authority_path),site=key,views=rows,scope='Exact geometry and saved materials; editable only unowned front texels and unknown reverse; closed solid alpha remains unchanged. No geometry or native texture changes.'))
 prepare(dest,sha(dest/'model.blend'),dest/'private-inputs')
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
