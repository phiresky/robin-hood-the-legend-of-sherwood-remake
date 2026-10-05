"""Private rounded fallen timber authored from the lower stream artwork."""
import json,math,sys
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_inventory import inventory
from refinement_workspace import prepare,modified
ASSET='croisement03-stream-fallen-log';NODE='scenery-croisement03-stream-fallen-log'
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))
DOMAIN=[(592,907),(612,905),(639,891),(661,881),(690,875),(729,871),(750,868),(762,860),(769,857),(766,870),(752,879),(726,883),(691,887),(669,899),(640,909),(615,916),(595,921)]
RINGS=[(574,919,13),(595,914,13),(615,909,12),(640,900,12),(663,889,10),(687,881,8),(713,878,7),(738,875,6),(752,871,5),(764,864,4),(769,860,2.5)]
def main():
    root=OUT/'restart2/fallen-log-v2';root.mkdir(exist_ok=False)
    source=OUT/'baseline/covered.png';image=Image.open(source);domain=Image.new('L',image.size);ImageDraw.Draw(domain).polygon(DOMAIN,fill=255);domain.save(root/'observed-domain.png')
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    native['masks'].append(dict(index=131,layer=0,layer_index=131,png=str(root/'observed-domain.png'),box_top_left=[0,0],box_size=list(image.size),authored=True,mask_type=0,obstacle_indices=[]))
    write_json(root/'mask-inventory.json',native)
    write_json(root/'source-masks.json',dict(version=1,mask_inventory=str(root/'mask-inventory.json'),projections={'exterior':dict(state='Initial lower stream fallen timber; hand-traced native surface domain',source_sha256=sha(source),assignments=[dict(reviewed=True,source_node=NODE,mask_indices=[131],exclude_mask_indices=[64],exclusions_reviewed=True,exclusion_reason='Three native shrub64 pixels cross the source-traced trunk boundary; preserve foliage ownership.')])}))
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement03 Working']
    fitted=json.loads((OUT/'restart2/fallen-log-fit-v2/fit.json').read_text());vertices=fitted['vertices'];faces=fitted['faces']
    mesh=bpy.data.meshes.new('Bent tapered round fallen trunk');mesh.from_pydata(vertices,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh)
    topology=dict(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free();assert topology==dict(nonmanifold_edges=0,degenerate_faces=0)
    obj=bpy.data.objects.new('Lower stream fallen log',mesh);collection.objects.link(obj)
    for k,v in dict(source_node=NODE,asset_group=ASSET,asset_name='Stream Fallen Log',part_name='Bent main trunk').items():obj[k]=v
    mat=bpy.data.materials.new('Unknown fallen timber');mat.diffuse_color=(.42,.42,.42,1);mesh.materials.append(mat);mesh.uv_layers.new(name='UVMap')
    catalog=json.loads((OUT/'catalog.json').read_text());catalog['groups'].append(dict(id=ASSET,name='Stream Fallen Log',parts=[dict(node=NODE,name='Bent main trunk')]))
    write_json(root/'catalog.json',catalog);inventory(root/'inventory',collection_name=collection.name,map_name='Croisement03',source_path=source)
    write_json(root/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(root/'catalog.json'),inventory_sha256=sha(root/'inventory/inventory.json'),evidence='Own native artwork and explicit source trace; authored scenery only, no native obstacle invented.'))
    bpy.ops.wm.save_as_mainfile(filepath=str(root/'input.blend'));worker=root/'assets'/ASSET
    prepare(worker,asset_id=ASSET,scene_name='Croisement03 Refinement',collection_name=collection.name,source_path=source,grouping_manifest=root/'catalog.json',inventory_path=root/'inventory/inventory.json',review_path=root/'grouping-review.json',source_mask_manifest=root/'source-masks.json',width=256,height=256,framing_padding=1.2,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    modified(worker);(worker/'inspection').mkdir(exist_ok=True)
    write_json(worker/'inspection/construction.json',dict(status='PRIVATE HOLD: source and actual materials review pending',model_sha256=sha(worker/'model.blend'),topology=topology,source_trace_rings=RINGS,fitted_parameters=fitted['parameters'],limitations=['Source trace and bent main axis are observations; hidden back surface, end caps and height above water are inferred.','Left end continues under foreground vegetation instead of stopping at observed texture boundary.','Dark narrow feature below the central trunk is treated as water shadow pending contrary evidence.','No native obstacle or collision added. Joint bank, rock and water contact remains unfinished.']))
    release();print(worker)
if __name__=='__main__':main()
