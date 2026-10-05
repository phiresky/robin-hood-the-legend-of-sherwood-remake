"""Continue a reviewed hall footprint to its verified native ground receiver."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('workspace',type=Path)
parser.add_argument('output',type=Path)
parser.add_argument('--support-evidence',type=Path,required=True)
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
if args.output.exists():raise FileExistsError(args.output)
evidence=json.loads(args.support_evidence.read_text())
samples=evidence['foundation_vertical_samples']
if len(samples)<3 or any(r['ground_world'] is None for r in samples):
    raise ValueError('Foundation continuation requires complete measured ground samples')
if any(abs(r['ground_world'][2])>.01 or r['vertical_gap_world']<10 for r in samples):
    raise ValueError('This control applies only to the verified flat ground gap')
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT/'tooling/current.json').read_text())['directory'])
import bpy
import bmesh
from source_projection_bake import bake

source=args.workspace/'model.blend'
bpy.ops.wm.open_mainfile(filepath=str(source))
config=json.loads((args.workspace/'workspace.json').read_text())
scene=bpy.data.scenes[config['scene_name']];bpy.context.window.scene=scene
bpy.context.view_layer.update();working=bpy.data.collections[config['collection_name']]
original=[o for o in working.all_objects if o.type=='MESH']
def fingerprint(obj):
    return {'points':[list(obj.matrix_world@v.co) for v in obj.data.vertices],
            'faces':[list(f.vertices) for f in obj.data.polygons],
            'uv':[[list(d.uv) for d in l.data] for l in obj.data.uv_layers],
            'materials':[m.name if m else None for m in obj.data.materials],
            'hidden':obj.hide_render}
before={o.name:fingerprint(o) for o in original}
matches=[o for o in original if o.get('source_node')=='building-769'
         and o.get('asset_group')==config['asset_id']
         and o.get('projection_component')=='hall' and not o.hide_render]
if len(matches)!=1:raise ValueError('Expected one existing hall body before foundation continuation')
body=matches[0];points=[body.matrix_world@v.co for v in body.data.vertices]
minimum=min(v.z for v in points)
bottoms=[p for p in body.data.polygons if all(abs(points[i].z-minimum)<.001 for i in p.vertices)]
if len(bottoms)!=1:raise ValueError('Expected one complete existing foundation footprint')
top=[points[i] for i in bottoms[0].vertices];n=len(top)
vertices=[(p.x,p.y,0) for p in top]+[tuple(p) for p in top]
faces=[tuple(reversed(range(n))),tuple(n+i for i in range(n))]
faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
mesh=bpy.data.meshes.new('Great hall native-ground foundation continuation')
mesh.from_pydata(vertices,[],faces)
bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
if any(not e.is_manifold for e in bm.edges):raise ValueError('Open foundation continuation')
bm.to_mesh(mesh);bm.free();mesh.uv_layers.new(name='UVMap')
obj=bpy.data.objects.new('Castle great hall / Foundation continuation',mesh)
working.objects.link(obj);obj['source_node']='building-769'
obj['projection_component']='hall-foundation';obj['asset_group']=config['asset_id']
obj['refinement_note']='Continues the saved hall footprint to verified native ground; the raised town receiver does not reach the castle.'
bpy.context.view_layer.update();args.output.mkdir(parents=True)
bake('york',Path(config['source_path']),args.output/'projection.json',receiver_nodes=['building-769'],
     receiver_asset_id=config['asset_id'],receiver_object_names=[obj.name],
     projection_label='private-native-ground-foundation',texels_per_unit=2,preserve_authored=False)
if before!={o.name:fingerprint(o) for o in original}:raise ValueError('Foundation changed an existing receiver')
bpy.context.preferences.filepaths.save_version=0
bpy.ops.wm.save_as_mainfile(filepath=str(args.output/'model.blend'),compress=True)
(args.output/'workspace.json').write_text(json.dumps(config,indent=2)+'\n')
(args.output/'geometry.json').write_text(json.dumps({
    'status':'HOLD: foundation continuation awaiting native and original-terrain visual review',
    'source_model_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
    'support_evidence_sha256':hashlib.sha256(args.support_evidence.read_bytes()).hexdigest(),
    'existing_geometry_uv_materials_preserved':len(original),
    'source_node':'building-769','projection_component':'hall-foundation',
    'top_world_z':minimum,'bottom_world_z':0,'footprint_world':[list(v) for v in top],
    'scope':'A closed continuation below the prior body; preserves every existing face and known material. No native obstacle or collision behavior is added.'},indent=2)+'\n')
