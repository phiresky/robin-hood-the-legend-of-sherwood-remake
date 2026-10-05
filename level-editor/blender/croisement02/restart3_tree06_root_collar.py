"""Private closed lower-root continuation constrained by the native wood outline."""
import sys,json,math
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release
from restart3_tree06_root_correction import fingerprint


def main():
 base=OUT/'restart3-tree06-root';dest=base/'collar-v3';dest.mkdir(exist_ok=False)
 probe=json.loads((base/'probe.json').read_text());source=Path(probe['model']);assert sha(source)==probe['model_sha256']
 bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update()
 existing=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-06']
 before={o.name:fingerprint(o) for o in existing}
 row=next(r for r in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks']if r['index']==6)
 mask=np.asarray(Image.open(OUT/'baseline/masks'/row['png']).convert('L'))>0
 yy,xx=np.nonzero(mask);cells={(int(x+row['box_top_left'][0]),int(y+row['box_top_left'][1]))for x,y in zip(xx,yy)if y+row['box_top_left'][1]>=515}
 # Cell boundaries retain the measured silhouette. Smooth depth across the
 # entire collar gives a rounded solid with an inferred buried underside.
 def corner_key(p,cell):
  x,y=p;around={(x-dx,y-dy)for dx,dy in [(0,0),(1,0),(1,1),(0,1)]}&cells
  diagonal=len(around)==2 and len({a[0]for a in around})==2 and len({a[1]for a in around})==2
  return (*p,*cell) if diagonal else (*p,-1,-1)
 keys=sorted({corner_key((x+dx,y+dy),(x,y))for x,y in cells for dx,dy in [(0,0),(1,0),(1,1),(0,1)]})
 corners=[p[:2]for p in keys];index={p:i for i,p in enumerate(keys)};count=len(corners)
 boundary=[p for p in corners if sum((p[0]-dx,p[1]-dy)in cells for dx,dy in [(0,0),(1,0),(1,1),(0,1)])<4]
 edge=np.asarray(boundary,float);vertices=[];bank_z=43.9495
 heights=[]
 for x,y in corners:
  distance=float(np.min(np.linalg.norm(edge-[x,y],axis=1)))
  # Sourceward radius rises smoothly toward the root centre and its trunk
  # connection. Depth, unlike source coordinates, is inferred.
  height=bank_z+1.2+min(17.,1.55*distance)+max(0,529-y)*.22
  heights.append(height)
  vertices.append((x,(-y-COS*height)/SIN,height))
 for (x,y),height in zip(corners,heights):
  buried=bank_z-4.-min(5.,(height-bank_z)*.35)
  vertices.append((x,(-y-COS*buried)/SIN,buried))
 faces=[];roles=[]
 for x,y in sorted(cells):
  ids=[index[corner_key(p,(x,y))]for p in [(x,y),(x+1,y),(x+1,y+1),(x,y+1)]]
  faces.extend([(ids[0],ids[2],ids[1]),(ids[0],ids[3],ids[2]),tuple(i+count for i in ids)])
  roles.extend([0,0,1])
  for a,b,neighbor in [(ids[0],ids[1],(x,y-1)),(ids[1],ids[2],(x+1,y)),(ids[2],ids[3],(x,y+1)),(ids[3],ids[0],(x-1,y))]:
   if neighbor not in cells:faces.append((a,b,b+count,a+count));roles.append(1)
 mesh=bpy.data.meshes.new('Tree06 source-constrained root collar');mesh.from_pydata(vertices,[],faces);mesh.update()
 obj=bpy.data.objects.new('Northwest Tree 06 / Root collar continuation',mesh);bpy.context.scene.collection.objects.link(obj)
 template=next(o for o in existing if o.name.endswith('wood 057'))
 for key in template.keys():obj[key]=template[key]
 obj['part_name']='Inferred complete root collar';obj['source_node']='building-057';obj['root_continuation']='Native wood6 lower contour; inferred bank contact and buried depth'
 image=bpy.data.images.load(str(OUT/'baseline/covered.png'),check_existing=False);image.pack()
 mat=bpy.data.materials.new('Tree06 collar / observed native front');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();links=mat.node_tree.links
 uv=nodes.new('ShaderNodeUVMap');uv.uv_map='Root native projection';tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest'
 emission=nodes.new('ShaderNodeEmission');output=nodes.new('ShaderNodeOutputMaterial')
 links.new(uv.outputs['UV'],tex.inputs['Vector']);links.new(tex.outputs['Color'],emission.inputs['Color']);links.new(emission.outputs[0],output.inputs[0])
 mat['texture_provenance']='Observed native wood6 source; hidden depth is inferred'
 unknown=bpy.data.materials.new('Tree06 collar / inferred underside pending fill');unknown.diffuse_color=(.18,.18,.18,1);unknown.use_nodes=True
 mesh.materials.append(mat);mesh.materials.append(unknown);layer=mesh.uv_layers.new(name='Root native projection')
 for face,role in zip(mesh.polygons,roles):
  face.material_index=role
  for loop in face.loop_indices:
   x,y=corners[mesh.loops[loop].vertex_index%count];layer.data[loop].uv=(x/1792,1-y/1152)
 bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));nonmanifold=sum(not e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free()
 assert nonmanifold==0,nonmanifold
 assert all(fingerprint(o)==before[o.name] for o in existing)
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'))
 target={tuple(r['pixel'])for r in probe['rays']}|{tuple(p)for p in probe['missing']};assert target<=cells
 write_json(dest/'report.json',dict(status='Private closed root continuation; visual/contact/native first-hit review pending',input_model=str(source),input_sha256=sha(source),model_sha256=sha(dest/'model.blend'),changed_object=obj.name,unchanged_objects=list(before),unchanged_existing_geometry_uv_material_transform=True,source_pixels=len(cells),target313_source_cells_covered=len(target),source_mask_sha256=sha(OUT/'baseline/masks'/row['png']),vertices=len(vertices),faces=len(faces),nonmanifold_edges=nonmanifold,inferred_front_z=[min(heights),max(heights)],limitations=['Native wood outline is measured; root depth and bank penetration are inferred.','Inferred underside remains neutral pending reviewed geometry and texture workflow.','Existing components remain unchanged; source first-hit and joint contact still need verification.','New geometry has no inherited approval.']))
 print(dest,flush=True)


if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
