from pathlib import Path
import json,hashlib,sys,math
import bpy,numpy as np
from PIL import Image
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_multiview_asset import render
from refinement_workspace import _geometry
O=R/'level-editor/work/croisement02-refinement';D=O/'restart6-tree07-bark-close-v1';W=O/'root-stem-round-2/assets/croisement02-tree-07';h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();read=lambda p:json.loads(p.read_text());expected='8bcb2ea9a6c40f920e15590569934f18c3bb3c002c26bb8160a6903605f107df';assert h(W/'model.blend')==expected
parent=read(O/'restart6-tree07-bark-input-v1/input-review.json');assert parent['approved_model_sha256']==expected
acquire()
try:
 bpy.ops.wm.open_mainfile(filepath=str(W/'model.blend'));meta=read(W/'modified/views.json');scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;bpy.context.view_layer.update();objects=[bpy.data.objects[n] for n in meta['object_names'] if 'wood ' in n];editnames=[o.name for o in objects if o.name.endswith(('wood 058','wood 062'))];before={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.type=='MESH'};stem=next(o for o in objects if o.name.endswith('wood 058'));points=[stem.matrix_world@v.co for v in stem.data.vertices];bounds=np.array(points);center=Vector((bounds.min(0)+bounds.max(0))/2);meta['object_names']=[o.name for o in objects];meta['render_object_names']=meta['object_names'];meta['texture_receiver_object_names']=editnames;meta['framing']='Tight lower wood058 framing; surrounding wood shown, crown excluded for material inspection only. Original native35deg view first.';meta['source_blend']=str(W/'model.blend');meta['known_rule']='Fresh authoritative source projection ownership masks, then restrict unknown to058/062. All other stored appearance protected.'
 for v in meta['views']:
  matrix=Matrix(v['camera_matrix_world']);back=matrix.col[2].to_3d();matrix.translation=center+back*5000;inverse=matrix.inverted();projected=[inverse@p for p in points];span=max(max(p.x for p in projected)-min(p.x for p in projected),max(p.y for p in projected)-min(p.y for p in projected));v.update(camera_matrix_world=[list(r) for r in matrix],camera_location=list(matrix.translation),ortho_scale=span*1.38,crop=dict(left=0,top=0,width=384,height=384))
 (D/'views.json').write_text(json.dumps(meta,indent=2)+'\n');scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;hidden_before={o.name:o.hide_render for o in scene.objects};display=dict(meta);display.pop('texture_receiver_object_names');display.pop('render_object_names');
 for ob in scene.objects:
  if ob.type=='MESH' and ob.get('asset_group')==meta['asset_id'] and ob.name not in meta['object_names']:ob.hide_render=True
 (D/'display-views.json').write_text(json.dumps(display,indent=2)+'\n');
 if not (D/'actual/view-7-solid.png').exists():render(D/'display-views.json',D/'actual',modes=('textured','solid'),width=384)
 for ob in scene.objects:
  if ob.name in hidden_before:ob.hide_render=hidden_before[ob.name]
 from refinement_review import render_review
 sourceframe=dict(meta);sourceframe['render_object_names']=sorted(meta['object_names'])
 if not (D/'source/views.json').exists():render_review(D/'source',scene_name=meta['scene_name'],collection_name=meta['collection_name'],asset_id=meta['asset_id'],source_path=W/'reference/source.png',frame_manifest=sourceframe,projection_layers=meta['projection_layers'],source_mask_manifest=meta['source_mask_manifest'],render_object_names=sorted(meta['object_names']))
 verts=[];faces=[];records=[];images={}
 for ob in objects:
  ob.data.calc_loop_triangles();offset=len(verts);verts.extend(ob.matrix_world@v.co for v in ob.data.vertices);uv=ob.data.uv_layers['Owned source / exterior']
  for t in ob.data.loop_triangles:
   faces.append([offset+i for i in t.vertices]);records.append((ob.name,t.material_index,[ob.matrix_world@ob.data.vertices[i].co for i in t.vertices],[Vector((*uv.data[i].uv,0)) for i in t.loops]))
  if ob.name in editnames:
   for i,mat in enumerate(ob.data.materials):
    if not mat or not mat.use_nodes:continue
    nodes=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image]
    if mat.get('source_ownership_alpha'):
     assert len(nodes)==1;im=nodes[0].image;data=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(data);images[(ob.name,i)]=data.reshape(im.size[1],im.size[0],4)
 tree=BVHTree.FromPolygons(verts,faces,all_triangles=True);assert images;sheet=Image.new('RGBA',(1536,768));masks=Image.new('RGBA',(1536,768));(D/'views').mkdir(exist_ok=True);reports=[]
 for v in meta['views']:
  i=v['index'];actual=np.array(Image.open(D/f'actual/view-{i}-textured.png').convert('RGBA'));m=Matrix(v['camera_matrix_world']);right=m.col[0].to_3d();up=m.col[1].to_3d();direction=-m.col[2].to_3d();unit=v['ortho_scale']/384;edit=np.zeros((384,384),bool);sourceknown=np.array(Image.open(D/f'source/views/view-{i}-known.png').convert('RGBA'))[:,:,0]>127
  for y in range(384):
   for x in range(384):
    p,n,f,dist=tree.ray_cast(m.translation+right*((x+.5-192)*unit)+up*((192-y-.5)*unit),direction)
    if p is None:continue
    name,slot,world,uv=records[f]
    if (name,slot) not in images:continue
    im=images[(name,slot)];hh,ww=im.shape[:2];q=barycentric_transform(p,*world,*uv);tx=min(ww-1,max(0,int(q.x*ww)));ty=min(hh-1,max(0,int(q.y*hh)))
    # Protect bilinear footprint and neighboring atlas texels, including native gray.
    observed=sourceknown[max(0,y-1):min(384,y+2),max(0,x-1):min(384,x+2)].any()
    edit[y,x]=not observed
  guide=actual.copy();guide[edit,:3]=77;mask=guide.copy();mask[edit,3]=0;known=np.full_like(actual,255);known[edit,:3]=0
  for name,arr in [('input',guide),('mask',mask),('known',known)]:Image.fromarray(arr).save(D/f'views/view-{i}-{name}.png')
  sheet.paste(Image.fromarray(guide),((i%4)*384,(i//4)*384));masks.paste(Image.fromarray(mask),((i%4)*384,(i//4)*384));reports.append(dict(view=i,editable_pixels=int(edit.sum()),protected_rgba_exact=bool(np.array_equal(guide[~edit],actual[~edit])),alpha_exact=bool(np.array_equal(guide[:,:,3],actual[:,:,3]))))
 sheet.save(D/'input.png');masks.save(D/'mask.png');actualsheet=Image.new('RGBA',(1536,768));solidsheet=Image.new('RGBA',(1536,768))
 for i in range(8):
  actualsheet.paste(Image.open(D/f'actual/view-{i}-textured.png'),((i%4)*384,(i//4)*384));solidsheet.paste(Image.open(D/f'actual/view-{i}-solid.png'),((i%4)*384,(i//4)*384))
 actualsheet.save(D/'actual.png');solidsheet.save(D/'solid.png');refs=read(O/'restart6-tree07-bark-input-v1/auxiliary-references.json');refs.update(input_sha256=h(D/'input.png'),lighting_sha256=h(D/'solid.png'));(D/'auxiliary-references.json').write_text(json.dumps(refs,indent=2)+'\n');assert before=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.type=='MESH'};assert h(W/'model.blend')==expected
 proof=dict(status='Prepared new framing for grouped input review; no API',model=str(W/'model.blend'),model_sha256=expected,geometry_decision=parent['decision'],editable_receivers=editnames,source_ownership='Fresh source projection and mask constraints; source-known pixels and one-pixel neighborhood protected irrespective RGB. Legacy neutral atlas alpha opaque, not used as ownership.',original_native_camera_first=True,geometry_materials_unchanged=True,views=reports,limitations=['Crown omitted from material close-up only, remains fully protected in model.','Upper wood060/061 preserved; input repair limited058/062, final combined low-angle saved-model review required.','New framing/input awaits grouped review before API.'],files={str(p.relative_to(D)):h(p) for p in D.rglob('*') if p.is_file() and p.name not in ('input-review.json','run.log','self-review.json','ready-candidate-v1.json')});(D/'input-review.json').write_text(json.dumps(proof,indent=2)+'\n');print(json.dumps(reports),flush=True)
finally:release()
