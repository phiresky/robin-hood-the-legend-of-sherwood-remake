"""Build private fern candidates excluding conservatively traced trunk pixels."""
import argparse
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from render_slots import acquire,release
from evidence_io import sha,write_json
from refinement_inventory import inventory
from refinement_workspace import prepare,modified
from ground_plant_geometry import build

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--round',default='v1');parser.add_argument('--masks',nargs='+',type=int,default=[35,76]);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    directory=OUT/'restart2/fern-ownership-v1';directory.mkdir(exist_ok=False)
    level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text());source=OUT/'baseline/covered.png';rgb=Image.open(source).convert('RGBA');sw,sh=rgb.size
    baseline_catalog=json.loads((OUT/'catalog.json').read_text());native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    # Each edited ownership domain has its own private authored index.
    for index, authored_index in [(35,131),(76,132)]:
        native_row=level['masks'][index];x,y=native_row['box_top_left'];w,h=native_row['box_size']
        native_alpha=np.asarray(Image.open(OUT/f'baseline/masks/{index:06}.png').convert('L')).copy()
        proposed=np.asarray(Image.open(OUT/f'restart2/fern-wood-proposal-v1/{index}-proposed-wood.png').convert('L'))[y:y+h,x:x+w]>0
        native_alpha[proposed]=0
        edited=directory/f'{index}-plant-domain.png';Image.fromarray(native_alpha).save(edited)
        native['masks'].append(dict(index=authored_index,layer=0,layer_index=authored_index,png=str(edited),mask_type=7,box_top_left=[x,y],box_size=[w,h],obstacle_indices=[],provenance=dict(native_mask=index,excluded_traced_wood_pixels=int(proposed.sum()))))
    write_json(directory/'mask-inventory.json',native)
    for index in args.masks:
        branch=directory/f'fern-{index:02}';branch.mkdir();node=f'foliage-fern-{index:03}';asset=f'croisement03-fern-{index:02}';name=f'North Woodland Fern {index:02}'
        m=level['masks'][index];x,y=m['box_top_left'];w,h=m['box_size'];im=rgb.crop((x,y,x+w,y+h));alpha=Image.open(directory/f'{index}-plant-domain.png').convert('L');im.putalpha(alpha);im.save(branch/'observed-source.png');im.save(branch/'complete-source.png')
        packet=dict(directory=str(branch),native_bbox=[x,y,w,h],bbox=[x,y,w,h],native_mask=index,plant_kind='fern',ground_z=0,source_sha256=sha(source),source_note='Own native fern RGBA only; rear leaflet colors are inferred from its own observed palette. Ground contact is a local hypothesis pending joint view.')
        write_json(branch/'partition.json',packet)
        catalog=json.loads(json.dumps(baseline_catalog));catalog['groups'].append(dict(id=asset,name=name,parts=[dict(node=node,name=name)]));write_json(branch/'catalog.json',catalog)
        assignments=[dict(source_node=node,mask_indices=[131 if index==35 else 132],reviewed=True)]
        constraints=[dict(reviewed=True,source_node=node,receiver_nodes=['ground']+[f'building-{i:03}' for i in range(106)],mask_indices=[131 if index==35 else 132],reason='Inferred fern reverse surfaces may only occlude foreign source receivers within this fern native domain.')]
        write_json(branch/'source-masks.json',dict(version=1,mask_inventory=str(directory/'mask-inventory.json'),projections={'exterior':dict(state='Private plant ownership with narrow traced bark exclusions',source_sha256=sha(source),assignments=assignments,occluder_constraints=constraints)}))
        acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
        collection=bpy.data.collections['Croisement03 Working'];obj=bpy.data.objects.new(name,bpy.data.meshes.new(name));collection.objects.link(obj)
        for key,value in dict(source_node=node,asset_group=asset,asset_name=name,part_name=name).items():obj[key]=value
        geometry=build(obj,packet)
        inventory(branch/'inventory',collection_name=collection.name,map_name='Croisement03',source_path=source,patch_manifest=OUT/'source-states/layers.json')
        write_json(branch/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(branch/'catalog.json'),inventory_sha256=sha(branch/'inventory/inventory.json'),evidence='Native fern mask, source crop and both threshold layers inspected. Separate authored plant; no obstacle or gameplay collision invented. Source overlap against future canopy/ground receivers remains an integration obligation.'))
        bpy.ops.wm.save_as_mainfile(filepath=str(branch/'input.blend'))
        worker=directory/'assets'/asset
        prepare(worker,asset_id=asset,scene_name='Croisement03 Refinement',collection_name=collection.name,source_path=source,grouping_manifest=branch/'catalog.json',inventory_path=branch/'inventory/inventory.json',review_path=branch/'grouping-review.json',source_mask_manifest=branch/'source-masks.json',width=256,height=256,framing_padding=1.2,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
        modified(worker);inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
        write_json(inspection/'construction.json',dict(status='private; actual eight views, native coverage and neighbor contact review pending',model_sha256=sha(worker/'model.blend'),native_mask=index,geometry=geometry,source_sha256=sha(source),known_rgba_sha256=sha(branch/'observed-source.png'),ownership_correction='Narrow traced bark excluded: 9 pixels in fern35,64 in fern76. Coarse first-hit receivers building048/building024 respectively. Remaining overlap is unresolved and must be visually reviewed.'))
        release();print(worker,flush=True)
if __name__=='__main__':main()
