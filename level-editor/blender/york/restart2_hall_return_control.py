"""Bounded hall end-wall control excluding the unsupported obstacle return."""
import argparse, hashlib, json, math, shutil, sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--version',required=True);p.add_argument('--height',type=float,default=273);p.add_argument('--preserve-coplanar',action='store_true');args=p.parse_args(sys.argv[sys.argv.index('--')+1:])
out=BASE/'restart2'/args.version
if out.exists():raise FileExistsError(out)
if shutil.disk_usage(ROOT).free<25*1024**3:raise RuntimeError('Disk below25GiB')
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((BASE/'tooling/current.json').read_text())['directory'])
import bpy,bmesh
from mathutils import Matrix
from source_projection_bake import bake
from render_multiview_asset import render
source=BASE/'restart2/hall-textures-v1/applied-applied/bake-v1/model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.data.scenes['york Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update()
target=next(o for o in bpy.data.collections['york Working'].all_objects if o.get('source_node')=='building-791')
def fp(o):
 return hashlib.sha256(json.dumps({'v':[list(v.co) for v in o.data.vertices],'f':[list(f.vertices) for f in o.data.polygons],'matrix':[list(r) for r in o.matrix_world],'materials':[m.name if m else None for m in o.data.materials],'uv':[[list(d.uv) for d in layer.data] for layer in o.data.uv_layers],'hidden':o.hide_render},sort_keys=True).encode()).hexdigest()
outside={o.name:fp(o) for o in scene.objects if o.type=='MESH' and o!=target}
level=json.loads((BASE/'baseline/york.rhp.json').read_text());profile=level['sight_obstacles'][791]['points'];plan=[(profile[i]['x'],profile[i]['y']) for i in [0,1,6,7]]
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));v=[(x,-y/s,z/c) for z in [225.001,args.height] for x,y in plan];faces=[(3,2,1,0),(4,5,6,7)]+[(i,(i+1)%4,(i+1)%4+4,i+4) for i in range(4)]
previous=target.data
mesh=bpy.data.meshes.new('Hall northwest low return control');mesh.from_pydata(v,[],faces);bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));volume=abs(bm.calc_volume(signed=True));assert all(e.is_manifold for e in bm.edges) and volume>0;bm.to_mesh(mesh);bm.free();mesh.uv_layers.new(name='UVMap');target.data=mesh;target.parent=None;target.matrix_world=Matrix.Identity(4);bpy.context.view_layer.update();out.mkdir(parents=True)
if args.preserve_coplanar:
 from mathutils import Vector
 from mathutils.geometry import closest_point_on_tri, barycentric_transform
 previous.calc_loop_triangles()
 for layer in previous.uv_layers:
  if layer.name not in mesh.uv_layers:mesh.uv_layers.new(name=layer.name)
 for material in previous.materials:mesh.materials.append(material)
 gray=bpy.data.materials.new('New791 cut surfaces pending texture');gray.diffuse_color=(.18,.18,.18,1);mesh.materials.append(gray)
 transferred=[]
 for face in mesh.polygons:
  face.material_index=len(mesh.materials)-1
  candidates=[f for f in previous.polygons if f.normal.dot(face.normal)>.99999 and abs(f.normal.dot(face.center-previous.vertices[f.vertices[0]].co))<.005]
  for original in candidates:
   tris=[t for t in previous.loop_triangles if t.polygon_index==original.index]
   points=[face.center]+[mesh.vertices[mesh.loops[li].vertex_index].co for li in face.loop_indices];samples=[]
   for point in points:
    best=min([((point-closest_point_on_tri(point,*[previous.vertices[i].co for i in t.vertices])).length,t) for t in tris],key=lambda row:row[0])
    if best[0]>.005:break
    t=best[1];uvs=[previous.uv_layers.active.data[i].uv for i in t.loops];samples.append(barycentric_transform(point,*[previous.vertices[i].co for i in t.vertices],*[Vector((u.x,u.y,0)) for u in uvs]))
   if len(samples)==len(points):
    face.material_index=original.material_index
    for layer in previous.uv_layers:
     for li,point in zip(face.loop_indices,points[1:]):
      t=min(tris,key=lambda tri:(point-closest_point_on_tri(point,*[previous.vertices[i].co for i in tri.vertices])).length)
      uvs=[layer.data[i].uv for i in t.loops];uv=barycentric_transform(point,*[previous.vertices[i].co for i in t.vertices],*[Vector((u.x,u.y,0)) for u in uvs]);mesh.uv_layers[layer.name].data[li].uv=uv.xy
    transferred.append({'new_face':face.index,'original_face':original.index});break
 (out/'coplanar-material-transfer.json').write_text(json.dumps({'scope':'Existing appearance transferred only on coincident planar faces for inspection; new faces neutral','faces':transferred},indent=2)+'\n')
else:
 bake('york',BASE/'restart2/hall-cover-source-combinations-v1/patch001-applied_patch002-applied.png',out/'source-projection.json',receiver_nodes=['building-791'],receiver_asset_id='york-castle-great-hall',projection_label='hall-semantic-applied-applied',source_mask_manifest=BASE/'restart2/hall-source-authority-v1/source-masks.json',texels_per_unit=2,preserve_authored=False)
assert outside=={o.name:fp(o) for o in scene.objects if o.type=='MESH' and o!=target}
bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True)
(out/'geometry.json').write_text(json.dumps({'status':'Private geometry hypothesis; requires review','changed_source_nodes':['building-791'],'plan_game':plan,'bottom_game':225.001,'top_game':args.height,'closed_volume':volume,'outside_geometry_uv_materials_preserved':len(outside),'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((out/'model.blend').read_bytes()).hexdigest(),'inference':'Remove unsupported northwest obstacle spur. End-wall top datum is a bounded control for the observed hall/keep boundary, not an exact source measurement.','appearance':'Only791 source projection renewed. Other existing textures retained as inspection context, not new approval.'},indent=2)+'\n')
render(BASE/'restart2/hall-textures-v1/applied-applied/experiment/views.json',out/'actual',width=384)
