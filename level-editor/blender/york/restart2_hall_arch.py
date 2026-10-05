"""Author a closed stone arch receiver from the native entrance contour."""
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
destination=OUT/'restart2/hall-arch-v1'
if destination.exists():
    raise FileExistsError(destination)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT/'tooling/current.json').read_text())['directory'])
import bpy
import bmesh
from source_projection_bake import bake

source=OUT/'restart2/hall-joint-revealed-v2/model.blend'
source_image=OUT/'restart2/hall-cover-source-combinations-v1/patch001-applied_patch002-applied.png'
bpy.ops.wm.open_mainfile(filepath=str(source))
scene=bpy.data.scenes['york Refinement'];bpy.context.window.scene=scene
bpy.context.view_layer.update()
working=bpy.data.collections['york Working']
original=[o for o in working.all_objects if o.type=='MESH']
groups={'york-castle-main-keep','york-castle-east-round-tower','york-castle-great-hall'}
def shape(obj):
    return [list(obj.matrix_world@v.co) for v in obj.data.vertices],[list(f.vertices) for f in obj.data.polygons]
def surface(obj):
    return [[list(d.uv) for d in l.data] for l in obj.data.uv_layers],[m.name if m else None for m in obj.data.materials],obj.hide_render
before={o.name:shape(o) for o in original}
outside={o.name:surface(o) for o in original if o.get('asset_group') not in groups}
outer=[(2761,598),(2761,555),(2764,545),(2770,537),(2777,531),
       (2784,529),(2791,529),(2797,534),(2797,586)]
inner=[(2767,592),(2767,555),(2770,549),(2775,544),(2780,541),
       (2785,539),(2789,540),(2791,546),(2791,585)]
if len(outer)!=len(inner):
    raise ValueError('Arch contours must have matching stations')
# The disappearing rectangular cover establishes the entrance wall datum.
# The revealed arch remains scenery, with no invented native obstacle ID.
slope=(812.1484-820.30066)/(2792.2637-2765.1155)
def datum_y(x):return 820.30066+slope*(x-2765.1155)
sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
n=len(outer);vertices=[];game_profiles=[]
for depth in (0,1):
    for contour in (outer,inner):
        game=[]
        for i,(x,pixel_y) in enumerate(contour):
            y=datum_y(x)
            z=225.001 if i in (0,n-1) else y-pixel_y
            x-=depth*3.8205;y-=depth*4.18561
            game.append((x,y,z));vertices.append((x,-y/sine,z/cosine))
        game_profiles.append(game)
faces=[]
for i in range(n-1):
    faces += [(i,i+1,n+i+1,n+i),
              (2*n+i,3*n+i,3*n+i+1,2*n+i+1),
              (i,2*n+i,2*n+i+1,i+1),
              (n+i,n+i+1,3*n+i+1,3*n+i)]
for i in (0,n-1):faces.append((i,n+i,3*n+i,2*n+i))
mesh=bpy.data.meshes.new('Great hall entrance arch receiver')
mesh.from_pydata(vertices,[],faces)
bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
if any(not e.is_manifold for e in bm.edges):raise ValueError('Open arch receiver')
volume=abs(bm.calc_volume(signed=True))
if volume<1:raise ValueError('Degenerate arch receiver')
bm.to_mesh(mesh);bm.free();mesh.uv_layers.new(name='UVMap')
obj=bpy.data.objects.new('Castle great hall / Northwest stone arch',mesh)
working.objects.link(obj)
obj['source_node']='scenery-york-great-hall-northwest-arch'
obj['asset_group']='york-castle-great-hall'
obj['source_mask_reference']=646
obj['refinement_note']='Native entrance contour; static scenery receiver behind the independently removable keep cover.'
bpy.context.view_layer.update();destination.mkdir(parents=True)
for group in sorted(groups):
    nodes=sorted({o['source_node'] for o in working.all_objects if o.type=='MESH' and o.get('asset_group')==group and not o.hide_render})
    bake('york',source_image,destination/(group+'-projection.json'),
         receiver_nodes=nodes,receiver_asset_id=group,projection_label='private-entrance-arch',
         texels_per_unit=2,preserve_authored=False)
if before!={o.name:shape(o) for o in original}:raise ValueError('Arch changed existing world geometry')
if outside!={o.name:surface(o) for o in original if o.get('asset_group') not in groups}:raise ValueError('Arch changed unrelated receivers')
bpy.context.preferences.filepaths.save_version=0
bpy.ops.wm.save_as_mainfile(filepath=str(destination/'model.blend'),compress=True)
config=json.loads((OUT/'restart2/hall-joint-revealed-v2/workspace.json').read_text())
config['part_ids']=list(config.get('part_ids',[]))+[obj['source_node']]
(destination/'workspace.json').write_text(json.dumps(config,indent=2)+'\n')
(destination/'geometry.json').write_text(json.dumps({
    'status':'HOLD: authored arch hypothesis awaiting actual eight-view and native joint inspection',
    'source_model_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
    'source_image_sha256':hashlib.sha256(source_image.read_bytes()).hexdigest(),
    'outer_source_contour':outer,'inner_source_contour':inner,
    'source_node':obj['source_node'],'native_mask_reference':646,
    'closed_volume_world':volume,'game_profiles':game_profiles,
    'existing_geometry_preserved':len(original),'outside_materials_uv_visibility_preserved':len(outside),
    'inferred':['Arch depth uses the neighboring native cover wall datum.','Contour vertices trace the revealed stone arch; jamb bases continue to the room floor.'],
    'remaining':['Check source contour fit and attachment to retained wall791.','Check doorway clearance and whether the retained rear return needs further refinement.','Covered-state and independent cover combinations remain separate integration work.']},indent=2)+'\n')
