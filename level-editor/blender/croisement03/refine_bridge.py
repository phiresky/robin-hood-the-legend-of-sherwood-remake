"""Construct a private timber bridge hypothesis from numbered native observations."""
import json
import math
import sys
from pathlib import Path
import bpy
import bmesh
from mathutils import Vector
from PIL import Image, ImageDraw, ImageChops

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from trace_bridge import RUNS
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_inventory import inventory
from refinement_workspace import prepare,modified

SIN=math.sin(math.radians(35)); COS=math.cos(math.radians(35))
ASSET='croisement03-timber-bridge'; NODE='scenery-croisement03-timber-bridge'
DECK_Z=32.0

def world(pixel,z):
    x,y=pixel
    return Vector((x,-(y+z*COS)/SIN,z))

def beam(verts,faces,a,b,width,depth=None):
    depth=width if depth is None else depth
    axis=(b-a).normalized(); side=axis.cross(Vector((0,0,1)))
    if side.length<.01:side=Vector((1,0,0))
    side.normalize(); up=axis.cross(side).normalized()
    n=len(verts)
    for end in [a,b]:
        for s,t in [(-1,-1),(1,-1),(1,1),(-1,1)]:
            verts.append(tuple(end+side*s*width/2+up*t*depth/2))
    faces.extend(tuple(n+i for i in face) for face in [(3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)])

def rail_world(pixel,side):
    a,b=(RUNS['deck_perimeter'][0],RUNS['deck_perimeter'][3]) if side=='near' else (RUNS['deck_perimeter'][1],RUNS['deck_perimeter'][2])
    t=(pixel[0]-a[0])/(b[0]-a[0]);base_y=a[1]+t*(b[1]-a[1])
    return world(pixel,DECK_Z+(base_y-pixel[1])/COS)

def main():
    root=OUT/'bridge-candidate-v1';root.mkdir(exist_ok=False)
    masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in masks['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    source=Image.open(OUT/'baseline/covered.png');domain=Image.new('L',source.size)
    ImageDraw.Draw(domain).polygon(RUNS['deck_perimeter'],fill=255)
    level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text())
    for i in [110,111,112]:
        canvas=Image.new('L',source.size);canvas.paste(Image.open(OUT/f'baseline/masks/{i:06}.png'),level['masks'][i]['box_top_left']);domain=ImageChops.lighter(domain,canvas)
    domain_path=root/'bridge-domain.png';domain.save(domain_path)
    # Match the native manifest schema while identifying authored ownership.
    row=dict(index=131,layer=0,layer_index=max(r['layer_index'] for r in masks['masks'] if r['layer']==0)+1,png=str(domain_path),box_top_left=[0,0],box_size=list(source.size),authored=True,mask_type=0,character_polyline=None,projectile_polyline=None,obstacle_indices=[])
    masks['masks'].append(row);write_json(root/'mask-inventory.json',masks)
    write_json(root/'source-masks.json',dict(version=1,mask_inventory=str(root/'mask-inventory.json'),projections={'exterior':dict(state='Initial timber bridge; deck boundary is a reviewed source trace hypothesis',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,source_node=NODE,mask_indices=[131],exclude_mask_indices=[55],exclusions_reviewed=True,exclusion_reason='Native shrub55 overlaps the near southeast deck by146 pixels; retain its foreground ownership.')])}))
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement03 Working'];vertices=[];faces=[]
    corners=[world(p,DECK_Z) for p in RUNS['deck_perimeter']]
    # A continuous closed deck isolates structure first. Plank seams remain a
    # documented refinement obligation rather than an invented regular count.
    vertices.extend(tuple(p) for p in corners);vertices.extend(tuple(p-Vector((0,0,3))) for p in corners)
    faces.extend([(0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)])
    for side in ['near','far']:
        endpoints=[rail_world(p,side) for p in RUNS[side+'_handrail']];beam(vertices,faces,*endpoints,2.8,2.2)
        for name,pixels in RUNS.items():
            if name.startswith(side+'_post_'):
                a=rail_world(pixels[0],side);b=rail_world(pixels[1],side);beam(vertices,faces,a,b,3.1)
    # Pier and braces use a common projected support plane at the near edge.
    for name in ['pier','pier_brace_left','pier_brace_right']:
        pts=[rail_world(p,'near') for p in RUNS[name]];beam(vertices,faces,*pts,2.5)
    mesh=bpy.data.meshes.new('Native-traced timber bridge');mesh.from_pydata(vertices,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh)
    topology=dict(vertices=len(bm.verts),faces=len(bm.faces),open_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
    assert topology['open_edges']==0 and topology['degenerate_faces']==0
    obj=bpy.data.objects.new('Timber bridge',mesh);collection.objects.link(obj)
    for key,value in dict(source_node=NODE,asset_group=ASSET,asset_name='Timber Bridge',part_name='Deck, rails and pier').items():obj[key]=value
    mat=bpy.data.materials.new('Unknown bridge timber');mat.diffuse_color=(.42,.42,.42,1);mesh.materials.append(mat)
    catalog=json.loads((OUT/'catalog.json').read_text());catalog['groups'].append(dict(id=ASSET,name='Timber Bridge',parts=[dict(node=NODE,name='Deck, rails and pier')]))
    write_json(root/'catalog.json',catalog)
    bpy.ops.wm.save_as_mainfile(filepath=str(root/'bridge-grouped.blend'))
    inventory(root/'inventory',collection_name=collection.name,map_name='Croisement03',source_path=OUT/'baseline/covered.png')
    write_json(root/'grouping-review.json',dict(status='reviewed',catalog_sha256=sha(root/'catalog.json'),inventory_sha256=sha(root/'inventory/inventory.json'),evidence='New authored scenery node; no fabricated native obstacle. Preliminary bridge structure only.'))
    worker=root/'assets'/ASSET
    prepare(worker,asset_id=ASSET,scene_name='Croisement03 Refinement',collection_name=collection.name,source_path=OUT/'baseline/covered.png',grouping_manifest=root/'catalog.json',inventory_path=root/'inventory/inventory.json',review_path=root/'grouping-review.json',source_mask_manifest=root/'source-masks.json',width=256,height=256,framing_padding=1.2,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    modified(worker)
    write_json(worker/'construction.json',dict(model_sha256=sha(worker/'model.blend'),topology=topology,status='PRIVATE HOLD: source coverage, joints and plank refinement pending',limitations=['Deck height32 is inferred pending bank/water contact reconstruction.','Plank seams and far-end rail joints remain unfinished.','All closed members are assembled components; overlap at joints is intentional.','Pier foot and obscured southeast landing need terrain-neighbor review.']))
    release()

if __name__=='__main__':main()
