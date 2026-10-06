"""Prepare scoped bark views with exact saved native-texel ownership."""
from pathlib import Path
import sys,json,hashlib
import bpy,numpy as np
from PIL import Image
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_multiview_asset import render
from refinement_workspace import _geometry
O=R/'level-editor/work/croisement02-refinement';BASE=O/'restart6-five-bark-fill-v1';h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();read=lambda p:json.loads(p.read_text())
def reachable(material):
 root=next(n for n in material.node_tree.nodes if n.type=='OUTPUT_MATERIAL');pending=[root];seen=set();images=[]
 while pending:
  node=pending.pop()
  if node.name in seen:continue
  seen.add(node.name)
  if node.type=='TEX_IMAGE'and node.image:images.append(node)
  pending.extend(link.from_node for socket in node.inputs for link in socket.links)
 return images

def main(number):
 asset=f'croisement02-tree-{number:02d}';r=next(r for r in read(O/'restart6-source-coverage/approved-five-bark-scope-v1.json')['records']if r['asset_id']==asset);authority=next(r for r in read(BASE/'source-authority.json')['records']if r['asset']==asset);model=Path(r['model']);assert h(model)==r['model_sha256'];assert h(Path(r['geometry_approval_file']))==r['geometry_approval_sha256'];D=BASE/f'tree-{number:02d}-input-v2' if number==18 else BASE/f'tree-{number:02d}-input-v1';D.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(model));meta=read(model.parent/'cameras.json');scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;bpy.context.view_layer.update();wood=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset and 'crown'not in o.name.lower()];assert wood;before={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o.type=='MESH'};hidden={o.name:o.hide_render for o in scene.objects};names=[o.name for o in wood];points=[o.matrix_world@v.co for o in wood for v in o.data.vertices];coords=np.array(points);center=Vector((coords.min(0)+coords.max(0))/2)
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.name not in names
 meta.update(object_names=names,render_object_names=names,texture_receiver_object_names=names,source_blend=str(model),framing='Tight complete physical bark scope; unchanged crown excluded only from material views. Native35degree camera first.',known_rule='Exact original packed atlas provenance flag1 plus explicit own-native overlay alpha, with one-texel conservative neighborhood. Protect all native RGBA, no color threshold.')
 for v in meta['views']:
  m=Matrix(v['camera_matrix_world']);m.translation=center+m.col[2].to_3d()*5000;projected=[m.inverted()@p for p in points];span=max(max(p.x for p in projected)-min(p.x for p in projected),max(p.y for p in projected)-min(p.y for p in projected));m.translation+=(m.col[0].to_3d()*((max(p.x for p in projected)+min(p.x for p in projected))/2)+m.col[1].to_3d()*((max(p.y for p in projected)+min(p.y for p in projected))/2))*(1 if number==18 else 0);v.update(camera_matrix_world=[list(x)for x in m],camera_location=list(m.translation),ortho_scale=span*1.2,crop=dict(left=0,top=0,width=384,height=384))
 meta['tile_size']=[384,384];display=dict(meta);display.pop('render_object_names');display.pop('texture_receiver_object_names');(D/'display-views.json').write_text(json.dumps(display,indent=2)+'\n');scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;render(D/'display-views.json',D/'actual',modes=('textured','solid'),width=384)
 flags={digest:np.load(row['path'])['ownership']==1 for digest,row in authority['atlases'].items()};materials={};coverage=[]
 for ob in wood:
  for slot,mat in enumerate(ob.data.materials):
   if not mat or not any(p.material_index==slot for p in ob.data.polygons):continue
   constraints=[]
   for node in reachable(mat):
    im=node.image;digest=hashlib.sha256(bytes(im.packed_file.data)).hexdigest();uvlinks=[l.from_node.uv_map for l in node.inputs['Vector'].links if l.from_node.type=='UVMAP'];assert len(uvlinks)==1,(asset,mat.name,node.name,uvlinks);uvname=uvlinks[0]
    if digest in flags:known=flags[digest];kind='original-source-provenance'
    elif im.name.startswith(f'native{number}'):
     px=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(px);known=px.reshape(im.size[1],im.size[0],4)[:,:,3]>0;kind='explicit-native-overlay'
    else:raise ValueError(('Unrecognized reachable image ownership',asset,mat.name,im.name,digest))
    constraints.append((uvname,known));coverage.append(dict(object=ob.name,material_slot=slot,image=im.name,image_sha256=digest,uv=uvname,kind=kind,protected_texels=int(known.sum())))
   materials[(ob.name,slot)]=constraints
 verts=[];faces=[];records=[]
 for ob in wood:
  offset=len(verts);verts.extend(ob.matrix_world@v.co for v in ob.data.vertices);ob.data.calc_loop_triangles()
  for t in ob.data.loop_triangles:
   faces.append([offset+i for i in t.vertices]);uvs={u.name:[Vector((*u.data[l].uv,0))for l in t.loops]for u in ob.data.uv_layers};records.append((ob.name,t.material_index,[ob.matrix_world@ob.data.vertices[i].co for i in t.vertices],uvs))
 tree=BVHTree.FromPolygons(verts,faces,all_triangles=True);(D/'views').mkdir();sheet=Image.new('RGBA',(1536,768));mask_sheet=Image.new('RGBA',(1536,768));actualsheet=Image.new('RGBA',(1536,768));solid=Image.new('RGBA',(1536,768));reports=[]
 for v in meta['views']:
  i=v['index'];a=np.array(Image.open(D/f'actual/view-{i}-textured.png').convert('RGBA'));edit=np.zeros((384,384),bool);m=Matrix(v['camera_matrix_world']);right=m.col[0].to_3d();up=m.col[1].to_3d();direction=-m.col[2].to_3d();unit=v['ortho_scale']/384
  for y in range(384):
   for x in range(384):
    p,n,f,d=tree.ray_cast(m.translation+right*((x+.5-192)*unit)+up*((192-y-.5)*unit),direction)
    if p is None:continue
    name,slot,world,uvs=records[f];known=False
    for uv,owned in materials[(name,slot)]:
     q=barycentric_transform(p,*world,*uvs[uv]);hh,ww=owned.shape;tx=int(q.x*ww);ty=int(q.y*hh)
     if not(0<=tx<ww and 0<=ty<hh):
      if uv=='Owned source / exterior':known=True
      continue
     known|=bool(owned[max(0,ty-1):min(hh,ty+2),max(0,tx-1):min(ww,tx+2)].any())
    edit[y,x]=not known
  guide=a.copy();guide[edit,:3]=77;mask=guide.copy();mask[edit,3]=0;known=np.full_like(a,255);known[edit,:3]=0
  for name,arr in [('textured',guide),('known',known),('mask',mask)]:Image.fromarray(arr).save(D/f'views/view-{i}-{name}.png')
  pos=((i%4)*384,(i//4)*384);sheet.paste(Image.fromarray(guide),pos);mask_sheet.paste(Image.fromarray(mask),pos);actualsheet.paste(Image.fromarray(a),pos);solid.paste(Image.open(D/f'actual/view-{i}-solid.png'),pos);v['ownership_sha256']=h(D/f'views/view-{i}-known.png');reports.append(dict(view=i,editable_pixels=int(edit.sum()),protected_rgba_exact=bool(np.array_equal(a[~edit],guide[~edit])),alpha_exact=bool(np.array_equal(a[:,:,3],guide[:,:,3]))))
 for name,im in [('input.png',sheet),('mask.png',mask_sheet),('actual.png',actualsheet),('solid.png',solid)]:im.save(D/name)
 for ob in scene.objects:
  if ob.name in hidden:ob.hide_render=hidden[ob.name]
 assert before=={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o.type=='MESH'};assert h(model)==r['model_sha256'];(D/'views.json').write_text(json.dumps(meta,indent=2)+'\n');refs=read(O/'restart6-tree07-bark-close-v1/auxiliary-references.json');refs.update(input_sha256=h(D/'input.png'),lighting_sha256=h(D/'solid.png'));(D/'auxiliary-references.json').write_text(json.dumps(refs,indent=2)+'\n');proof=dict(status='Prepared scoped bark input; visual/input review pending',asset_id=asset,model=str(model),model_sha256=r['model_sha256'],geometry_approval=r,geometry_and_original_appearance_unchanged=True,authority_sha256=h(BASE/'source-authority.json'),material_ownership=coverage,views=reports,files={str(p.relative_to(D)):h(p)for p in D.rglob('*')if p.is_file()},limitations=['Existing source native gray/green remains protected even where visually unlike bark.','Unknown prior generated/reused bark may be replaced only within this displayed physical wood scope.','Crown geometry/material completely untouched; close-only exclusion.','Direct overlay alpha protected in addition to original atlas flag1.']);(D/'input-review.json').write_text(json.dumps(proof,indent=2)+'\n');print(asset,[(r['view'],r['editable_pixels'])for r in reports],flush=True)
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
