"""Sample approved bark fills without reprojection of legacy source artwork.

The generated atlas is an additional layer over immutable original UVs and
shaders. Original source provenance and native overlay alpha take precedence.
"""
from pathlib import Path
import sys,json,hashlib
import bpy,numpy as np
from PIL import Image,ImageFilter
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from render_slots import acquire,release
from bake_texture_candidate import preflight,snapshot
from source_projection_bake import bake
from project_reviewed_texture import _read,_reconcile
from restart6_bark_preservation import reachable_images,add_generated_layer
from render_multiview_asset import render
O=R/'level-editor/work/croisement02-refinement';B=O/'restart6-five-bark-fill-v1';read=lambda p:json.loads(p.read_text());h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def pixels(im):
 a=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(a);return a.reshape(im.size[1],im.size[0],4)
def image_array(name,a):
 im=bpy.data.images.new(name,width=a.shape[1],height=a.shape[0],alpha=True);im.pixels.foreach_set(a.astype(np.float32).ravel());im.update();im.pack();return im
def mask_image(name,a):
 a=np.asarray(Image.fromarray(np.uint8(a)*255).filter(ImageFilter.MaxFilter(3)))>0
 rgba=np.ones((*a.shape,4),np.float32);rgba[:,:,:3]=a[:,:,None];im=image_array(name,rgba);im.colorspace_settings.name='Non-Color';return im

def main(n):
 E=B/f'tree-{n}-fill-v1/experiment';D=B/f'tree-{n}-fill-v1/shader-restored-v3';assert not D.exists();meta,scene,names,baseline,pre=preflight(E);authority=next(r for r in read(B/'source-authority.json')['records']if r['asset']==meta['asset_id']);assert h(Path(authority['model']))==authority['model_sha256'];review=read(E/'generation-review.json');assert review['ready_for_bake'];raw=Path(review['raw_image']);gen=Path(review['preserved_image'])
 for p,key in [(raw,'raw_sha256'),(gen,'preserved_sha256'),(E/'input.png','input_sha256'),(E/'mask.png','mask_sha256'),(E/'approved-model.blend','model_sha256')]:assert h(p)==review[key]
 inp=_read(E/'input.png');pred=_read(gen);mask=_read(E/'mask.png')[:,:,3]==0;assert np.array_equal(inp[~mask],pred[~mask]);assert np.array_equal(inp[:,:,3],pred[:,:,3]);colorsheet=_reconcile(pred,dict(meta,texture_reconciliation_gain_mode='luminance'),_read(raw));height,width=mask.shape
 active=[scene.objects[x]for x in sorted(names)if not scene.objects[x].hide_render];assert active;active_names={o.name for o in active};original_uv={o.name:{uv.name:np.array([x.uv[:]for x in uv.data],np.float32)for uv in o.data.uv_layers}for o in active};hidden={o.name:o.hide_render for o in scene.objects}
 vertices=[];triangles=[]
 for ob in active:
  offset=len(vertices);vertices.extend(ob.matrix_world@v.co for v in ob.data.vertices);ob.data.calc_loop_triangles();triangles.extend(tuple(offset+i for i in t.vertices)for t in ob.data.loop_triangles)
 tree=BVHTree.FromPolygons(vertices,triangles,all_triangles=True);cameras=[(v,Matrix(v['camera_matrix_world']).inverted(),Matrix(v['camera_matrix_world']).col[2].to_3d())for v in meta['views']];stats=dict(sampled=0,unsupported=0)
 def sample(obj,normal,positions,accepted,colors,*,face_index):
  filled=np.zeros(len(positions),bool)
  for score,v,inv,direction in sorted([(normal.dot(d),v,m,d)for v,m,d in cameras],key=lambda x:x[0],reverse=True):
   if score<=.05:continue
   ids=np.flatnonzero(~accepted&~filled)
   if not len(ids):break
   local=positions[ids]@np.asarray(inv.to_3x3()).T+np.asarray(inv.translation);crop=v['crop'];px=crop['left']+(.5+local[:,0]/v['ortho_scale'])*crop['width'];py=height-crop['top']-(.5-local[:,1]/v['ortho_scale'])*crop['height']
   for k,index in enumerate(ids):
    x,y=int(np.floor(px[k])),int(np.floor(py[k]));left,bottom=crop['left'],height-crop['top']-crop['height']
    if not(left<=x<left+crop['width']and bottom<=y<bottom+crop['height'])or not mask[y,x]:continue
    point=Vector(positions[index]);hit,_,_,_=tree.ray_cast(point+direction*100000,-direction)
    if hit is None or(hit-point).length>.02:continue
    fx,fy=px[k]-.5,py[k]-.5;x0,y0=int(np.floor(fx)),int(np.floor(fy));ax,ay=fx-x0,fy-y0;x0=max(left,min(left+crop['width']-1,x0));y0=max(bottom,min(bottom+crop['height']-1,y0));x1=min(left+crop['width']-1,x0+1);y1=min(bottom+crop['height']-1,y0+1)
    # Never blend protected/background pixels into a generated bark sample.
    weights=np.array([(1-ax)*(1-ay),ax*(1-ay),(1-ax)*ay,ax*ay]);coords=[(y0,x0),(y0,x1),(y1,x0),(y1,x1)];weights*=np.array([mask[yy,xx]for yy,xx in coords]);total=weights.sum()
    if total<=0:continue
    colors[index,:3]=sum(colorsheet[yy,xx,:3]*w for(yy,xx),w in zip(coords,weights))/total;filled[index]=True
  stats['sampled']+=int(filled.sum());stats['unsupported']+=int((~accepted&~filled).sum());return filled
 D.mkdir();Image.new('RGBA',(1,1),(0,0,0,0)).save(D/'no-source.png')
 for ob in scene.objects:
  if ob.type=='MESH':ob.hide_render=ob.name not in active_names
 sampling_collection=bpy.data.collections.new('Private bark sampling scope');scene.collection.children.link(sampling_collection)
 for ob in active:sampling_collection.objects.link(ob)
 report=bake('Croisement02',D/'no-source.png',D/'generated-atlas.json',receiver_nodes=sorted({o.get('source_node')for o in active}),occluder_nodes=sorted({o.get('source_node')for o in active}),projection_label='reviewed-bark-generated-only',texels_per_unit=1,preserve_authored=False,hidden_sampler=sample,hidden_sampler_receives_face=True,collection_name=sampling_collection.name,receiver_object_names=sorted(active_names),provenance_directory=D/'provenance')
 captured={}
 for ob in active:
  row=next(x for x in report['objects']if x['object']==ob.name);p=Path(row['texel_provenance']['path']);assert h(p)==row['texel_provenance']['sha256'];flags=np.load(p)['ownership'];assert not(flags==1).any();uvname='Owned source / reviewed-bark-generated-only';uv=ob.data.uv_layers[uvname];mat=ob.data.materials[ob.data.polygons[0].material_index];im=next(x.image for x in mat.node_tree.nodes if x.type=='TEX_IMAGE');captured[ob.name]=(np.array([x.uv[:]for x in uv.data],np.float32),pixels(im),flags)
 bpy.ops.wm.open_mainfile(filepath=str(E/'approved-model.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;assert snapshot(scene,names)==baseline;protection={};source_images={};records=[]
 for name,(uvdata,rgba,flags)in captured.items():
  ob=scene.objects[name];uvname='Reviewed bark generated';assert not ob.data.uv_layers.get(uvname);uv=ob.data.uv_layers.new(name=uvname)
  for x,value in zip(uv.data,uvdata):x.uv=value
  genimage=image_array(name+' / reviewed bark',rgba);support=np.ones_like(rgba);support[:,:,:3]=(flags==2)[:,:,None];supportimage=image_array(name+' / generation support',support);supportimage.colorspace_settings.name='Non-Color';used={p.material_index for p in ob.data.polygons}
  for slot in sorted(used):
   mat=ob.data.materials[slot]
   for node in reachable_images(mat):
    im=node.image;digest=hashlib.sha256(bytes(im.packed_file.data)).hexdigest();source_images[im.name]=digest
    if digest in protection:continue
    if digest in authority['atlases']:
     ar=authority['atlases'][digest];p=Path(ar['path']);assert h(p)==ar['sha256'];known=np.load(p)['ownership']==1
    elif im.name.startswith(f'native{n}'):known=pixels(im)[:,:,3]>0
    else:raise ValueError(('Unknown source authority',im.name,digest))
    protection[digest]=mask_image(im.name+' / protected original',known)
   ob.data.materials[slot]=add_generated_layer(mat,genimage,uvname,supportimage,protection,name=mat.name+' / reviewed bark')
  for olduv,values in original_uv[name].items():assert np.array_equal(np.array([x.uv[:]for x in ob.data.uv_layers[olduv].data],np.float32),values)
  records.append(dict(object=name,generated_texels=int((flags==2).sum()),fallback_texels=int((flags!=2).sum()),original_uv_layers_exact=True))
 assert snapshot(scene,names)==baseline;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;bpy.ops.wm.save_as_mainfile(filepath=str(D/'worker.blend'));modelhash=h(D/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(D/'worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;assert snapshot(scene,names)==baseline
 for name,digest in source_images.items():assert hashlib.sha256(bytes(bpy.data.images[name].packed_file.data)).hexdigest()==digest
 close=dict(meta);close.pop('render_object_names',None);close.pop('texture_receiver_object_names',None);(D/'close-views.json').write_text(json.dumps(close,indent=2)+'\n');full=read(Path(authority['model']).parent/'cameras.json');full.pop('render_object_names',None);full.pop('texture_receiver_object_names',None);(D/'full-views.json').write_text(json.dumps(full,indent=2)+'\n')
 for scope in ['close','full']:
  render_hidden={o.name:o.hide_render for o in scene.objects}
  if scope=='close':
   for ob in scene.objects:
    if ob.type=='MESH'and ob.name not in active_names:ob.hide_render=True
  render(D/f'{scope}-views.json',D/scope,width=384);sheet=Image.new('RGBA',(1536,768))
  for i in range(8):sheet.paste(Image.open(D/scope/f'view-{i}-textured.png'),((i%4)*384,(i//4)*384))
  sheet.save(D/scope/'textured.png')
  for ob in scene.objects:ob.hide_render=render_hidden[ob.name]
 proof=dict(status='Saved candidate; actual review pending',asset_id=meta['asset_id'],model_sha256=modelhash,parent_model_sha256=authority['model_sha256'],geometry_and_outside_appearance_exact=True,all_original_images_packed_exact=True,all_original_uv_layers_exact=True,source_shader_preserved_as_fallback=True,protection='One-texel expanded original flag1 OR native overlay alpha; only generated support uses new atlas',source_images=source_images,active_receivers=sorted(active_names),retained_hidden_receivers=sorted(names-active_names),records=records,sampling=stats,reconciliation='Luminance-only preserves generated bark hue near protected gray/green source',files={str(p.relative_to(D)):h(p)for p in D.rglob('*')if p.is_file()});(D/'preservation.json').write_text(json.dumps(proof,indent=2)+'\n');print(modelhash,flush=True)
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
