"""Independently check saved fork continuity, native coverage, and retained surfaces."""
import sys,json,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree24_contour import ROOT,OUT,SPECS,covered
from approved_texture_stage import geometry,appearance
from evidence_io import sha,write_json,digest
from render_slots import acquire,release

def capture(path):
 bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-24'];wood=[o for o in objects if'Crown'not in o.name];rows=set();images={}
 for o in wood:
  for f in o.data.polygons:
   mat=o.data.materials[f.material_index]
   for node in mat.node_tree.nodes:
    if node.type=='TEX_IMAGE'and node.image and node.image.packed_file:images[node.image.name]=hashlib.sha256(bytes(node.image.packed_file.data)).hexdigest()
   for li in f.loop_indices:
    p=o.matrix_world@o.data.vertices[o.data.loops[li].vertex_index].co
    if 100<p.z<150:continue
    uv=tuple((u.name,tuple(round(x,6)for x in u.data[li].uv))for u in o.data.uv_layers if u.name!='Exact lower contour native projection');rows.add((tuple(round(x,4)for x in p),uv,mat.name))
 return objects,wood,rows,images,{o.name:dict(geometry=geometry(o),appearance=appearance(o))for o in objects if'Crown'in o.name}
acquire()
try:
 source=ROOT/'tree24-contour-v2/model.blend';version=int(sys.argv[sys.argv.index('--')+1]) if '--' in sys.argv and sys.argv[sys.argv.index('--')+1].isdigit() else 2;model=ROOT/f'tree24-fork-union-v{version}/model.blend';item=next(x for x in json.loads((OUT/'review-mask-inventory.json').read_text())['masks']if x['index']==24);ox,oy=item['box_top_left'];yy,xx=np.where(np.array(Image.open(item['png']))>0);targets=np.column_stack((xx+ox+.5,yy+oy+.5));a,w,old,images,crowns=capture(source);oldcov=covered(w,targets);b,w,new,newimages,newcrowns=capture(model);newcov=covered(w,targets);mask=np.array(Image.open(ROOT/'source-audit-v1/exposed-24.png'))>0;y,x=np.where(mask);targetcov=covered(w,np.column_stack((x+.5,y+.5)));topology=[]
 for o in w:
  bm=bmesh.new();bm.from_mesh(o.data);topology.append(dict(object=o.name,nonmanifold=sum(not e.is_manifold for e in bm.edges),degenerate=sum(f.calc_area()<1e-9 for f in bm.faces)));bm.free()
 result=dict(model_sha256=sha(model),parent_sha256=sha(source),crown_exact=crowns==newcrowns,original_packed_images_exact=all(newimages.get(k)==v for k,v in images.items()),outside_joint_rows_exact=old==new,missing_rows=len(old-new),extra_rows=len(new-old),native_before=int(oldcov.sum()),native_after=int(newcov.sum()),lost_native=(targets[oldcov&~newcov]-.5).tolist(),target_coverage=targetcov.tolist(),topology=topology)
 write_json(model.parent/'fork-guard.json',result);print(result,flush=True)
finally:release()
