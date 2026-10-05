"""Reconstruct the southwest firewood as closed logs using its native footprint."""
import json
import math
import sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw,ImageChops
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import prepare,modified
ASSET='croisement03-southwest-firewood-stack'
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))

def main():
    root=OUT/'restart2/firewood-v7';root.mkdir(parents=True,exist_ok=False)
    worker=root/'assets'/ASSET
    if worker.exists():raise FileExistsError(worker)
    masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in masks['masks']: row['png']=str(OUT/'baseline/masks'/row['png'])
    source=Image.open(OUT/'baseline/covered.png');domain=Image.new('L',source.size);draw=ImageDraw.Draw(domain)
    observed_faces=[[(420,760),(426,759),(431,762),(435,764),(440,767),(445,771),(450,774),(455,777),(454,780),(449,781),(445,779),(440,776),(436,773),(431,769),(426,766),(422,764)],[(431,776),(438,777),(441,782),(439,785),(435,786),(431,783)],[(442,778),(449,778),(448,782),(445,784),(442,784)]]
    for polygon in observed_faces:draw.polygon(polygon,fill=255)
    level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text());native=Image.new('L',source.size);native.paste(Image.open(OUT/'baseline/masks/000114.png'),level['masks'][114]['box_top_left']);domain=ImageChops.darker(domain,native);domain.save(root/'observed-wood-domain.png')
    masks['masks'].append(dict(index=131,layer=0,layer_index=131,png=str(root/'observed-wood-domain.png'),box_top_left=[0,0],box_size=list(source.size),authored=True,mask_type=0,obstacle_indices=[]))
    write_json(root/'source-trace.json',dict(native_mask=114,observed_faces=observed_faces,source_sha256=sha(OUT/'baseline/covered.png'),scope='Positive trace of clearly visible timber and pale billet ends. Native114 green background and unseen rear lengths remain unowned, not assigned to wood.'))
    write_json(root/'mask-inventory.json',masks)
    write_json(root/'source-masks.json',dict(version=1,mask_inventory=str(root/'mask-inventory.json'),projections={'exterior':dict(state='Initial native firewood; obscured timber remains unknown',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=ASSET,mask_indices=[131],exclude_mask_indices=[45,54],exclusions_reviewed=True,exclusion_reason='Native source close-up shows foreground leaf pixels over timber. Foliage45 covers487 firewood-mask pixels and foliage54 covers11; these remain foliage-owned, with neighboring plant geometry still required.')])}))
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    prepare(worker,asset_id=ASSET,scene_name='Croisement03 Refinement',collection_name='Croisement03 Working',source_path=OUT/'baseline/covered.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'inventory/inventory.json',review_path=OUT/'grouping-review.json',source_mask_manifest=root/'source-masks.json',width=256,height=256,framing_padding=1.20,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    obj=next(o for o in bpy.data.collections['Croisement03 Working'].all_objects if o.type=='MESH' and o.get('source_node')=='building-049')
    fit=json.loads((OUT/'restart2/firewood-fit-v6/fit.json').read_text())
    verts=fit['vertices'];faces=fit['faces'];logs=fit['logs']
    old=obj.data;mesh=bpy.data.meshes.new('Southwest firewood closed billets');mesh.from_pydata(verts,[],faces);mesh.update()
    material=bpy.data.materials.new('Unknown firewood surface')
    material.diffuse_color=(.42,.42,.42,1)
    mesh.materials.append(material)
    mesh.uv_layers.new(name='UVMap')
    obj.data=mesh;obj.matrix_world.identity()
    import bmesh
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh)
    topology=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
    assert topology['nonmanifold_edges']==0 and topology['degenerate_faces']==0
    modified(worker)
    inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    write_json(inspection/'construction.json',dict(status='private candidate; visual and source coverage review pending',model_sha256=sha(worker/'model.blend'),native_mask=114,native_obstacle=49,logs=logs,topology=topology,limitations=['Three billets and hidden end profiles are inferred; observed source axes and final image determine plausibility, not fitted silhouette alone.','Observed timber is restricted to a positive source trace inside native114. Background green and hidden rear lengths remain unowned.','Lighting is explicit provisional inspection lighting; map shadow calibration remains pending.']))
    release()
    print(worker)
if __name__=='__main__': main()
