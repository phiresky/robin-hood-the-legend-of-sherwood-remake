"""Prepare approved initial rig textures with explicit saved-material ownership."""
import sys,json,shutil
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
ROOT=OUT/'restart5-initial-nets'
EXPECTED={'00':'b9dbe2fc4ad3e1a83b25b7b5a2965c9c41eae55cef97545129a46c19e573ddab','01':'3b95c00c5a997d26b0699f14d345593e8e635d9bc2926502c519ae100ecb8f11'}
def main(key):
 assert shutil.disk_usage(OUT).free>25*1024**3
 base=ROOT/f'candidate-v5/profile-{key}';model=base/'model.blend';assert sha(model)==EXPECTED[key]
 approval=OUT/'restart3-review-batches/batch-v9/user-approval.json';assert sha(approval)=='f2fc3aea62a55c61a44d282d43e9543f9d53f3bd1fa7d57e9afe189c3a4d586c'
 report=json.loads((base/'report.json').read_text());dest=ROOT/f'texture-inputs-v1/profile-{key}';dest.mkdir(parents=True,exist_ok=False);asset='croisement02-initial-net-'+('01' if key=='00' else '03')
 bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();objects=sorted([o for o in scene.objects if o.type=='MESH'],key=lambda o:o.name);before=signature(objects);col=bpy.data.collections.new('Initial rig texture Working');scene.collection.children.link(col)
 canvas=Image.new('RGBA',Image.open(OUT/'baseline/covered.png').size);xy=report['source']['origin'];canvas.alpha_composite(Image.open(base/'observed-source.png').convert('RGBA'),tuple(xy));canvas.save(dest/'source.png');inventory=[];assignments=[]
 roles={'Ground camouflage net':'ground','Initial lifting line':'rope','Inferred upper fastening loop':None}
 for i,o in enumerate(objects):
  col.objects.link(o);o.hide_render=False;o['asset_group']=asset;o['source_node']=asset+f'-part-{i}';mask=Image.new('L',canvas.size)
  if roles[o.name]:mask.paste(Image.open(ROOT/f'source/profile-{key}-{roles[o.name]}-mask.png').convert('L'),tuple(xy))
  mask.save(dest/f'role-{i}.png');inventory.append(dict(index=i,box_top_left=[0,0],box_size=list(canvas.size),png=f'role-{i}.png'));assignments.append(dict(reviewed=True,source_node=o['source_node'],mask_indices=[i]))
 write_json(dest/'mask-inventory.json',dict(masks=inventory));write_json(dest/'source-masks.json',dict(version=1,mask_inventory='mask-inventory.json',projections=dict(exterior=dict(source_sha256=sha(dest/'source.png'),state='Approved initial h rig; uncertain high fragments unassigned',assignments=assignments))))
 assert signature(objects)==before;bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True)
 render_review(dest/'modified',scene_name=scene.name,collection_name=col.name,asset_id=asset,source_path=dest/'source.png',source_mask_manifest=dest/'source-masks.json',width=384,height=384,framing_padding=1.2)
 meta=json.loads((dest/'modified/views.json').read_text());meta['texture_material_suffix']='initial-rig-unknown-v1'
 for v in meta['views']:v['crop']={'left':0,'top':0,'width':384,'height':384}
 write_json(dest/'modified/views.json',meta);scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;render(dest/'modified/views.json',dest/'stored',width=384)
 points=[];triangles=[];records=[];native=np.asarray(Image.open(base/'observed-source.png').convert('RGBA'));ih,iw=native.shape[:2];uvname='Initial native source projection'
 for o in objects:
  o.data.calc_loop_triangles();offset=len(points);points.extend(o.matrix_world@v.co for v in o.data.vertices)
  for t in o.data.loop_triangles:
   triangles.append([offset+i for i in t.vertices]);mat=o.data.materials[t.material_index];known=any(n.type=='TEX_IMAGE' and n.image for n in mat.node_tree.nodes);records.append((known,[o.matrix_world@o.data.vertices[i].co for i in t.vertices],[Vector((*o.data.uv_layers[uvname].data[i].uv,0))for i in t.loops] if known else None))
 tree=BVHTree.FromPolygons(points,triangles,all_triangles=True);sheet=Image.new('RGBA',(1536,768));rows=[]
 for v in meta['views']:
  idx=v['index'];m=Matrix(v['camera_matrix_world']);unit=v['ortho_scale']/384;editable=np.zeros((384,384),bool)
  for y in range(384):
   for x in range(384):
    p,n,f,d=tree.ray_cast(m.translation+m.col[0].to_3d()*((x+.5-192)*unit)+m.col[1].to_3d()*((192-y-.5)*unit),-m.col[2].to_3d())
    if p is None:continue
    known,world,uv=records[f];observed=False
    if known:
     q=barycentric_transform(p,*world,*uv);tx=int(q.x*iw);ty=int((1-q.y)*ih);observed=0<=tx<iw and 0<=ty<ih and native[ty,tx,3]>0
    editable[y,x]=not observed
  actual=np.array(Image.open(dest/'stored'/f'view-{idx}-textured.png').convert('RGBA'));guide=actual.copy();guide[editable,:3]=77;known=np.full((384,384,4),255,np.uint8);known[editable,:3]=0
  Image.fromarray(guide).save(dest/'modified/views'/f'view-{idx}-textured.png');Image.fromarray(known).save(dest/'modified/views'/f'view-{idx}-known.png');v['ownership_sha256']=sha(dest/'modified/views'/f'view-{idx}-known.png');sheet.paste(Image.fromarray(guide),((idx%4)*384,(idx//4)*384));rows.append(dict(view=idx,editable_pixels=int(editable.sum()),protected_rgba_exact=bool(np.array_equal(actual[~editable],guide[~editable]))))
 sheet.save(dest/'modified/textured.png');write_json(dest/'modified/views.json',meta);assert signature(objects)==before;assert sha(model)==EXPECTED[key]
 write_json(dest/'derivation.json',dict(source_model=str(model),source_model_sha256=sha(model),prepared_model_sha256=sha(dest/'model.blend'),geometry_uv_material_signature=before,geometry_uv_materials_unchanged=True,source_image=str(base/'observed-source.png'),source_sha256=sha(base/'observed-source.png'),user_approval=str(approval),user_approval_sha256=sha(approval),object_names=[o.name for o in objects],native_uv=uvname,views=rows,scope='Exact geometry and saved materials; editable only native alpha-zero fallback and unknown reverse. High fragments remain unassigned.'))
 prepare(dest,sha(dest/'model.blend'),dest/'private-inputs')
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
